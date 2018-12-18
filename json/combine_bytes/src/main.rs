#[macro_use]
extern crate bencher;
#[macro_use]
extern crate combine;

use std::hash::Hash;
use std::result::Result::Ok;
use std::str;

use bencher::{Bencher, black_box};
use bytes::Bytes;
use combine::{
    any,
    error::{
        Consumed,
        Info,
        ParseError,
        ParseResult2,
        StreamError,
        Tracked
    },
    Parser,
    parser,
    parser::{
        byte::{byte, bytes, digit},
        choice::{choice, optional},
        combinator::{ignore, no_partial},
        item::{self, one_of, satisfy_map, tokens},
        range,
        repeat::{escaped, many, many1, sep_by},
        sequence::between,
    },
    ParseResult,
    RangeStream,
    satisfy,
    stream::{
        Range,
        StreamErrorFor,
    },
    Stream,
    StreamOnce,
};
use combine::range::TakeWhile1;
use combine::stream::wrap_stream_error;

use crate::{
    byterange::BytesBuf,
    byterange::BytesRange,
    byterange::skip_while,
    byterange::SkipRangeStream,
    byterange::SkipWhile
};
use crate::byterange::skip_while1;

pub mod byterange;

#[derive(PartialEq, Debug)]
enum Value {
    Number(Bytes),
    String(Bytes),
    Bool(bool),
    Null,
    Object(Vec<(Bytes, Value)>),
    Array(Vec<Value>),
}

fn spaces<I>() -> impl Parser<Input=I, Output=I::SkipValue>
    where
      I: SkipRangeStream<Item=u8, Range=BytesRange>,
      I::Error: ParseError<
          I::Item,
          I::Range,
          I::Position,
      >
{
    skip_while(|b| {
        b == b' ' || b == b'\t' || b == b'\r' || b == b'\n'
    })
}

fn lex<P>(p: P) -> impl Parser<Input=P::Input, Output=P::Output>
    where
      P: Parser,
      P::Input: SkipRangeStream<Item=u8, Range=BytesRange>,
      <P::Input as StreamOnce>::Error: ParseError<
          <P::Input as StreamOnce>::Item,
          <P::Input as StreamOnce>::Range,
          <P::Input as StreamOnce>::Position,
      >
{
    no_partial(spaces().with(p))
}

fn lex_around<P>(p: P) -> impl Parser<Input=P::Input, Output=P::Output>
    where
      P: Parser,
      P::Input: SkipRangeStream<Item=u8, Range=BytesRange>,
      <P::Input as StreamOnce>::Error: ParseError<
          <P::Input as StreamOnce>::Item,
          <P::Input as StreamOnce>::Range,
          <P::Input as StreamOnce>::Position,
      >
{
    (spaces(), p, spaces())
      //this doesn't look very good :)
      .map(|(_, o, _)| o)
}


fn digits<I>() -> impl Parser<Input=I, Output=I::SkipValue>
    where
      I: SkipRangeStream<Item=u8, Range=BytesRange>,
      I::Error: ParseError<I::Item, I::Range, I::Position>,
{
    skip_while1(|b| b >= b'0' && b <= b'9')
}

fn number<I>() -> impl Parser<Input=I, Output=BytesRange>
    where
      I: SkipRangeStream<Item=u8, Range=BytesRange>,
      I::Error: ParseError<I::Item, I::Range, I::Position>,
{
    no_partial(
        inspect("number", range::recognize(no_partial((
            optional(one_of("+-".bytes())),
            choice((
                (digits(), optional((byte(b'.'), optional(digits())))).map(|_| ()),
                (byte(b'.'), digits()).map(|_| ())
            )),
            optional((
                (one_of("eE".bytes()), optional(one_of("+-".bytes()))),
                digits(),
            )),
        ))))
    )
}

fn json_string<I>() -> impl Parser<Input=I, Output=BytesRange>
    where
      I: SkipRangeStream<Item=u8, Range=BytesRange>,
      I::Error: ParseError<I::Item, I::Range, I::Position>,
{
    let back_slash_byte = satisfy_map(|c| {
        Some(match c {
            b'"' => b'"',
            b'\\' => b'\\',
            b'/' => b'/',
            b'b' => '\u{0008}' as u8,
            b'f' => '\u{000c}' as u8,
            b'n' => b'\n',
            b'r' => b'\r',
            b't' => b'\t',
            _ => return None,
        })
    });
    let inner = range::recognize(escaped(
        skip_while1(|b| b != b'\\' && b != b'"'),
        b'\\',
        back_slash_byte,
    ));
    inspect("string", inner.skip(byte(b'"')))
}

fn object<I>() -> impl Parser<Input=I, Output=Vec<(Bytes, Value)>>
    where
      I: SkipRangeStream<Item=u8, Range=BytesRange>,
      I::Error: ParseError<I::Item, I::Range, I::Position>,
{
    let field = ((byte(b'"').with(json_string())).map(|bytes| bytes.0), lex(byte(b':')), json_value()).map(|t| (t.0, t.2));
    let fields = sep_by(field, lex_around(byte(b',')));
    inspect("object",
    between(spaces(), lex(byte(b'}')), fields))
}

fn array<I>() -> impl Parser<Input=I, Output=Vec<Value>>
    where
      I: SkipRangeStream<Item=u8, Range=BytesRange>,
      I::Error: ParseError<I::Item, I::Range, I::Position>,
{
    inspect(
        "array",
        sep_by(json_value(), lex(byte(b',')))
          .skip(lex(byte(b']'))),
    )
}

#[inline(always)]
fn json_value<I>() -> impl Parser<Input=I, Output=Value>
    where
      I: SkipRangeStream<Item=u8, Range=BytesRange>,
      I::Error: ParseError<I::Item, I::Range, I::Position>,
{
    lex_around(json_value_())
}


fn value<I>(buf: &'static str) -> impl Parser<Input=I, Output=()>
    where I: SkipRangeStream<Item=u8, Range=BytesRange>,
          I::Error: ParseError<I::Item, I::Range, I::Position> {
    inspect(buf, tokens(|l,r| l == &r, Info::Range(BytesRange(Bytes::from_static(buf.as_bytes()))), buf.as_bytes().iter())
      .map(|_| ()))
}

#[inline(always)]
fn inspect<P>(buf: &'static str, p: P) -> impl Parser<Input=P::Input, Output=P::Output>
    where P: Parser,
          P::Input: SkipRangeStream<Item=u8, Range=BytesRange>,
          <P::Input as StreamOnce>::Error: ParseError<
              <P::Input as StreamOnce>::Item,
              <P::Input as StreamOnce>::Range,
              <P::Input as StreamOnce>::Position> {
    
    p.expected(buf)
//    parser(move |input| {
//        //println!("Parser {}", buf);
//        Ok(((), Consumed::Empty(())))
//    }).with(p)
//      .expected(buf)
}
// We need to use `parser!` to break the recursive use of `value` to prevent the returned parser
// from containing itself
parser! {
    #[inline(always)]
    fn json_value_[I]()(I) -> Value
        where [ I: SkipRangeStream<Item = u8, Range = BytesRange> ]
    {
        parser(|mut i: &mut I| {
            //println!("Parser Value");
            let before = i.checkpoint();
            let x = match i.uncons() {
                Ok(b) => b,
                Err(e) => return wrap_stream_error(i, e).into()
            };
            match x {
                b'[' => array().map(Value::Array).parse_stream(i),
                b'{' => object().map(Value::Object).parse_stream(i),
                b'\"' => json_string().map(|b| Value::String(b.0)).parse_stream(i),
                b'f' => value("alse").map(|_| Value::Bool(false)).parse_stream(i),
                b't' => value("rue").map(|_| Value::Bool(true)).parse_stream(i),
                b'n' => value("ull").map(|_| Value::Null).parse_stream(i),
                b'0'...b'9' | b'+' | b'-' | b'.' => {
                    i.reset(before);
                    number().map(|n| Value::Number(n.0)).parse_stream(i)
                },
                t => return Err(Consumed::Consumed(Tracked::from(I::Error::from_error(i.position(), StreamError::unexpected_token(t))))),
            }
        })
    }
}

#[test]
fn json_test() {
    use self::Value::*;
    use bytes::Bytes;
    use self::BytesRange;
    let input = br#"{
        "array": [1, ""],
        "object" :  {},
        "number" : 3.14,
        "small_number": 0.59,
        "int": -100,
        "exp": -1e2,
        "exp_neg": 23E-2,
        "true": true,
        "false"  : false,
        "null" : null
    }"#;
    macro_rules! b {
        ($str:expr) => {
            Bytes::from_static($str)
        }
    }
    let result = json_value().easy_parse(BytesBuf::new(Bytes::from_static(&*input)));
    let expected = Object(
        vec![
            (b!(b"array"), Array(vec![Number(b!(b"1")), String(b!(b""))])),
            (b!(b"object"), Object(Vec::default())),
            (b!(b"number"), Number(b!(b"3.14"))),
            (b!(b"small_number"), Number(b!(b"0.59"))),
            (b!(b"int"), Number(b!(b"-100"))),
            (b!(b"exp"), Number(b!(b"-1e2"))),
            (b!(b"exp_neg"), Number(b!(b"23E-2"))),
            (b!(b"true"), Bool(true)),
            (b!(b"false"), Bool(false)),
            (b!(b"null"), Null),
        ].into_iter()
         .collect(),
    );
    match result {
        Ok((result, input)) => assert_eq!(result, expected),
        Err(e) => {
            //println!("{}\n{:?}", e, e);
            assert!(false);
        }
    }
}

#[test]
fn test() {
    let data = "  { \"a\"\t: 42,
  \"b\": [ \"x\", \"y\", 12 ] ,
  \"c\": { \"hello\" : \"world\"
  }
  }  ";
    //let data = include_str!("../../test.json");
    
    let mut parser = json_value();
    let result = parser.easy_parse(BytesBuf::new(Bytes::from_static(data.as_bytes())));
    //println!("test: {:?}", result);
    result.unwrap();
}

#[test]
fn apache_test() {
    let data = include_str!("../../apache_builds.json");
    let mut parser = json_value();
    let result = parser.easy_parse(BytesBuf::new (Bytes::from_static(data.as_bytes())));
    //println!("test: {:?}", result);
    result.unwrap();
}

fn parse(b: &mut Bencher, buffer: &'static str) {
    let mut parser = json_value();
    let bytes = Bytes::from(buffer.as_bytes());
    b.iter(|| {
        let buf = black_box(BytesBuf::new(bytes.clone()));
        
        let result = parser.easy_parse(buf).unwrap();
        black_box(result)
    });
}


fn basic(b: &mut Bencher) {
    let data = "  { \"a\"\t: 42,
  \"b\": [ \"x\", \"y\", 12 ] ,
  \"c\": { \"hello\" : \"world\"
  }
  }  ";

    b.bytes = data.len() as u64;
    parse(b, data)
}

fn data(b: &mut Bencher) {
    let data = include_str!("../../data.json");
    b.bytes = data.len() as u64;
    parse(b, data)
}

fn canada(b: &mut Bencher) {
    let data = include_str!("../../canada.json");
    b.bytes = data.len() as u64;
    parse(b, data)
}

fn apache(b: &mut Bencher) {
    let data = include_str!("../../apache_builds.json");
    b.bytes = data.len() as u64;
    parse(b, data)
}

//deactivating the "basic" benchmark because the parser fails on this one
//benchmark_group!(json, basic, data, apache, canada);
benchmark_group!(json, basic, data, apache, canada);
benchmark_main!(json);

/*
fn main() {
  loop {
    let data = include_bytes!("../../canada.json");
    root(data).unwrap();
  }
}
*/
