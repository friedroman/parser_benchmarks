#[macro_use]
extern crate bencher;
#[macro_use]
extern crate combine;

use std::hash::Hash;
use std::str;

use bencher::{Bencher, black_box};
use bytes::Bytes;
use combine::{
    any,
    error::Consumed,
    error::ParseError,
    error::ParseResult2,
    Parser,
    parser,
    parser::{
        byte::{byte, bytes, digit, spaces},
        choice::{choice, optional},
        combinator::no_partial,
        item::{one_of, satisfy_map},
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

use self::byterange::BytesRange;

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

fn lex<'a, P>(p: P) -> impl Parser<Input=P::Input, Output=P::Output>
    where
      P: Parser,
      P::Input: RangeStream<Item=u8, Range=BytesRange>,
      <P::Input as StreamOnce>::Error: ParseError<
          <P::Input as StreamOnce>::Item,
          <P::Input as StreamOnce>::Range,
          <P::Input as StreamOnce>::Position,
      >
{
    no_partial(p.skip(range::take_while(|b| {
        b == b' ' || b == b'\t' || b == b'\r' || b == b'\n'
    })))
}

fn digits<I>() -> TakeWhile1<I, fn(I::Item) -> bool>
    where
      I: RangeStream<Item=u8, Range=BytesRange>,
      I::Error: ParseError<I::Item, I::Range, I::Position>,
{
    range::take_while1(|b| b >= b'0' && b <= b'9')
}

fn number<I>() -> impl Parser<Input=I, Output=BytesRange>
    where
      I: RangeStream<Item=u8, Range=BytesRange>,
      I::Error: ParseError<I::Item, I::Range, I::Position>,
{
    no_partial(
        lex(range::recognize(no_partial((
            optional(one_of("+-".bytes())),
            choice((
                (digits(), optional((byte(b'.'), optional(digits())))).map(|_| ()),
                (byte(b'.'), digits()).map(|_| ())
            )),
            optional((
                (one_of("eE".bytes()), optional(one_of("+-".bytes()))),
                digits(),
            )),
        )))).expected("number"),
    )
}

fn json_string<I>() -> impl Parser<Input=I, Output=BytesRange>
    where
      I: RangeStream<Item=u8, Range=BytesRange>,
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
        range::take_while1(|b| b != b'\\' && b != b'"'),
        b'\\',
        back_slash_byte,
    ));
    between(lex(byte(b'"')), lex(byte(b'"')), inner).expected("string")
}

fn object<I>() -> impl Parser<Input=I, Output=Vec<(Bytes, Value)>>
    where
      I: RangeStream<Item=u8, Range=BytesRange>,
      I::Error: ParseError<I::Item, I::Range, I::Position>,
{
    let field = (json_string().map(|bytes| bytes.0), lex(byte(b':')), json_value()).map(|t| (t.0, t.2));
    let fields = sep_by(field, lex(byte(b',')));
    between(lex(byte(b'{')), lex(byte(b'}')), fields).expected("object")
}

fn array<I>() -> impl Parser<Input=I, Output=Vec<Value>>
    where
      I: RangeStream<Item=u8, Range=BytesRange>,
      I::Error: ParseError<I::Item, I::Range, I::Position>,
{
    between(
        lex(byte(b'[')),
        lex(byte(b']')),
        sep_by(json_value(), lex(byte(b','))),
    ).expected("array")
}

#[inline(always)]
fn json_value<I>() -> impl Parser<Input=I, Output=Value>
    where
      I: RangeStream<Item=u8, Range=BytesRange>,
      I::Error: ParseError<I::Item, I::Range, I::Position>,
{
    lex(json_value_())
}

// We need to use `parser!` to break the recursive use of `value` to prevent the returned parser
// from containing itself
parser! {
    #[inline(always)]
    fn json_value_[I]()(I) -> Value
        where [ I: RangeStream<Item = u8, Range = BytesRange> ]
    {
        choice((
            json_string().map(|b| Value::String(b.0)),
            object().map(Value::Object),
            array().map(Value::Array),
            number().map(|b| Value::Number(b.0)),
            range::range(BytesRange(Bytes::from_static(b"false"))).map(|_| Value::Bool(false)),
            range::range(BytesRange(Bytes::from_static(b"true"))).map(|_| Value::Bool(true)),
            range::range(BytesRange(Bytes::from_static(b"null"))).map(|_| Value::Null),
        ))
    }
}

#[test]
fn json_test() {
    use self::Value::*;
    use bytes::Bytes;
    use self::BytesRange;
    let input = br#"{
        "array": [1, ""],
        "object": {},
        "number": 3.14,
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
    let result = json_value().easy_parse(BytesRange(Bytes::from_static(&*input)));
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
        Ok(result) => assert_eq!(result, (expected, BytesRange(Bytes::new()))),
        Err(e) => {
            println!("{}\n{:?}", e, e);
            assert!(false);
        }
    }
}

fn parse(b: &mut Bencher, buffer: &str) {
    let mut parser = json_value();
    b.iter(|| {
        let buf = black_box(BytesRange(Bytes::from(buffer.as_bytes())));

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

#[test]
fn test() {
    let data = "  { \"a\"\t: 42,
  \"b\": [ \"x\", \"y\", 12 ] ,
  \"c\": { \"hello\" : \"world\"
  }
  }  ";
    //let data = include_str!("../../test.json");

    let mut parser = json_value();
    let result = parser.easy_parse(BytesRange(Bytes::from_static(data.as_bytes())));
    println!("test: {:?}", result);
    result.unwrap();
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
