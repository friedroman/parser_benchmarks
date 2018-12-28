#![feature(existential_type)]
#[macro_use]
extern crate bencher;
#[macro_use]
extern crate combine;

use std::{
    hash::Hash,
    marker::PhantomData,
    str
};

use bencher::{Bencher, black_box};
use bytes::Bytes;
use combine::{
    any,
    combinator::{
        between, choice,
        escaped, ignore, many, many1, no_partial, one_of, optional,
        ParserSequenceState, PartialState2, PartialState3,
        satisfy_map, sep_by, SequenceState, tokens, Y
    },
    ConsumedResult,
    easy,
    Parser,
    parser,
    parser::{
        byte::{byte, bytes, digit},
        FirstMode,
        item::{self},
        ParseMode,
        PartialMode,
        range
    },
    ParseResult,
    range::TakeWhile1,
    RangeStream,
    satisfy,
    stream::{
        decode,
        Range,
        StreamErrorFor,
        wrap_stream_error
    },
    Stream,
    StreamOnce,
};
use combine::error::Info;
use combine::error::Tracked;
use combine::ParseError;
use combine::parser::combinator::any_partial_state;
use combine::parser::combinator::AnyPartialState;

use crate::{
    byterange::{
        BytesBuf,
        BytesRange,
        skip_while,
        skip_while1,
        SkipRangeStream,
        SkipWhile
    }
};

pub mod byterange;

#[derive(PartialEq, Debug)]
pub enum Value {
    Number(Bytes),
    String(Bytes),
    Bool(bool),
    Null,
    Object(Vec<(Bytes, Value)>),
    Array(Vec<Value>),
}

fn spaces<I>() -> impl Parser<Input=I, Output=I::SkipValue, PartialState=usize>
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

type SkipSequence<P> = SequenceState<<<P as Parser>::Input as SkipRangeStream>::SkipValue, usize>;
type LexState<P> = PartialState2<SequenceState<(), usize>, ParserSequenceState<P>>;

fn lex<P>(p: P) -> impl Parser<Input=P::Input, Output=P::Output, PartialState=LexState<P>>
    where
      P: Parser,
      P::Input: SkipRangeStream<Item=u8, Range=BytesRange>,
      <P::Input as StreamOnce>::Error: ParseError<
          <P::Input as StreamOnce>::Item,
          <P::Input as StreamOnce>::Range,
          <P::Input as StreamOnce>::Position,
      >
{
    spaces().with(p)
}


type LexArState<P> = PartialState3<
    SkipSequence<P>,
    ParserSequenceState<P>,
    SkipSequence<P>
>;

fn lex_around<P>(p: P) -> impl Parser<Input=P::Input, Output=P::Output, PartialState=LexArState<P>>
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


fn digits<I>() -> impl Parser<Input=I, Output=I::SkipValue, PartialState=()>
    where
      I: SkipRangeStream<Item=u8, Range=BytesRange>,
      I::Error: ParseError<I::Item, I::Range, I::Position>,
{
    skip_while1(|b| b >= b'0' && b <= b'9')
}

fn number<I>() -> impl Parser<Input=I, Output=BytesRange, PartialState=()>
    where
      I: SkipRangeStream<Item=u8, Range=BytesRange>,
      I::Error: ParseError<I::Item, I::Range, I::Position>,
{
        inspect("number", no_partial(range::recognize((
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
}

type StringState = PartialState3<SequenceState<u8, ()>, SequenceState<BytesRange, ()>, SequenceState<u8, ()>>;

fn json_string<I>() -> impl Parser<Input=I, Output=BytesRange, PartialState=StringState>
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
    inspect("string", between(byte(b'"'), byte(b'"'), inner))
}

type SkipSequenceInput<I> = SequenceState<<I as byterange::SkipRangeStream>::SkipValue, usize>;

//type ObjectState<I> = PartialState3<
//    SkipSequenceInput<I>,
//    SequenceState<
//        Vec<(Bytes, Value)>,
//        Y<(
//            Option<Consumed<()>>,
//            Vec<(Bytes, Value)>,
//            PartialState2<
//                SequenceState<
//                    (),
//                    PartialState3<
//                        SkipSequenceInput<I>,
//                        SequenceState<u8, ()>,
//                        SkipSequenceInput<I>
//                    >>,
//                SequenceState<
//                    (Bytes, Value),
//                    PartialState3<
//                        SequenceState<
//                            Bytes,
//                            PartialState2<
//                                SequenceState<(), ()>,
//                                SequenceState<BytesRange,StringState>
//                            >
//                        >,
//                        SequenceState<
//                            u8,
//                            PartialState2<
//                                SequenceState<(), usize>,
//                                SequenceState<u8, ()>
//                            >
//                        >,
//                        SequenceState<
//                            Value,
//                            JsonValueState<I>
//                        >
//                    >
//                >
//            >
//        ), ()>
//    >,
//    SequenceState<u8, PartialState2<SequenceState<(), usize>, SequenceState<u8, ()>>>
//>;

fn object<I>() -> impl Parser<Input=I, Output=Vec<(Bytes, Value)>, PartialState=AnyPartialState>
    where
      I: SkipRangeStream<Item=u8, Range=BytesRange> + 'static,
      I::Error: ParseError<I::Item, I::Range, I::Position>,
      <I as byterange::SkipRangeStream>::SkipValue: 'static
{
    let field = (json_string().map(|bytes| bytes.0), lex(byte(b':')), json_value()).map(|t| (t.0, t.2));
    let fields = sep_by(field, lex_around(byte(b',')));
    inspect("object",
    any_partial_state(between(byte(b'{').skip(spaces()), lex(byte(b'}')), fields)))
}

fn array<I>() -> impl Parser<Input=I, Output=Vec<Value>, PartialState=AnyPartialState>
    where
      I: SkipRangeStream<Item=u8, Range=BytesRange> + 'static,
      I::Error: ParseError<I::Item, I::Range, I::Position>,
      <I as byterange::SkipRangeStream>::SkipValue: 'static
{
    inspect(
        "array",
        any_partial_state(
            between(byte(b'['), lex(byte(b']')),
            sep_by(json_value(), lex(byte(b',')))))
    )
}

type LexValueState<I> = PartialState3<
    SequenceState<<I as SkipRangeStream>::SkipValue, usize>,
    SequenceState<Value, ValueState<I>>,
    SequenceState<<I as SkipRangeStream>::SkipValue, usize>,
>;

#[inline(always)]
fn json_value<I>() -> impl Parser<Input=I, Output=Value>
    where
      I: SkipRangeStream<Item=u8, Range=BytesRange> + 'static,
      <I as SkipRangeStream>::SkipValue: 'static,
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
fn inspect<P>(buf: &'static str, p: P) -> impl Parser<Input=P::Input, Output=P::Output, PartialState=P::PartialState>
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
//parser! {
//    #[derive(Clone)]
//    pub struct JsonValue;
//    type PartialState = ValueState;
//    pub fn json_value_[I]()(I) -> Value
//        where [ I: SkipRangeStream<Item = u8, Range = BytesRange> ]
//    {
////        parser(|mut i: &mut I| {
////            //println!("Parser Value");
////            let before = i.checkpoint();
////            let x = match i.uncons() {
////                Ok(b) => b,
////                Err(e) => return wrap_stream_error(i, e).into()
////            };
////            match x {
////                b'[' => array().map(Value::Array).parse_stream(i),
////                b'{' => object().map(Value::Object).parse_stream(i),
////                b'\"' => json_string().map(|b| Value::String(b.0)).parse_stream(i),
////                b'f' => value("alse").map(|_| Value::Bool(false)).parse_stream(i),
////                b't' => value("rue").map(|_| Value::Bool(true)).parse_stream(i),
////                b'n' => value("ull").map(|_| Value::Null).parse_stream(i),
////                b'0'...b'9' | b'+' | b'-' | b'.' => {
////                    i.reset(before);
////                    number().map(|n| Value::Number(n.0)).parse_stream(i)
////                },
////                t => return Err(Consumed::Consumed(Tracked::from(I::Error::from_error(i.position(), StreamError::unexpected_token(t))))),
////            }
////        })
//        choice((
//            json_string().map(|b| Value::String(b.0)),
//            object().map(Value::Object),
//            array().map(Value::Array),
//            number().map(|b| Value::Number(b.0)),
//            value("false").map(|_| Value::Bool(false)),
//            value("true").map(|_| Value::Bool(true)),
//            value("null").map(|_| Value::Null),
//        ))
//    }
//}

#[derive(Clone)]
pub struct JsonValue<I>
    where <I as StreamOnce>::Error:
    ParseError<
        <I as StreamOnce>::Item,
        <I as StreamOnce>::Range,
        <I as StreamOnce>::Position
    >,
          I: SkipRangeStream<Item=u8, Range=BytesRange> + 'static,
          <I as SkipRangeStream>::SkipValue: 'static
{
    __marker: PhantomData<fn(I) -> Value>
}

existential type ValueState<I>: Default;

fn value_choice<I>() -> impl Parser<Input=I, Output=Value, PartialState=ValueState<I>>
    where <I as StreamOnce>::Error:
    ParseError<
        <I as StreamOnce>::Item,
        <I as StreamOnce>::Range,
        <I as StreamOnce>::Position
    >,
          I: SkipRangeStream<Item=u8, Range=BytesRange> + 'static,
    <I as SkipRangeStream>::SkipValue: 'static

{
    choice((
        json_string().map(|b| Value::String(b.0)),
        object().map(Value::Object),
        array().map(Value::Array),
        number().map(|b| Value::Number(b.0)),
        value("false").map(|_| Value::Bool(false)),
        value("true").map(|_| Value::Bool(true)),
        value("null").map(|_| Value::Null),
    ))
}

impl<I> Parser for JsonValue<I>
    where <I as StreamOnce>::Error:
    ParseError<
        <I as StreamOnce>::Item,
        <I as StreamOnce>::Range,
        <I as StreamOnce>::Position
    >,
          I: SkipRangeStream<Item=u8, Range=BytesRange> + 'static,
          <I as SkipRangeStream>::SkipValue: 'static
{
    type Input = I;
    type Output = Value;
    type PartialState = ValueState<I>;
    
    #[inline(always)]
    fn parse_first(
        &mut self,
        input: &mut Self::Input,
        state: &mut Self::PartialState,
    ) -> ConsumedResult<Self::Output, Self::Input> {
        self.parse_mode(FirstMode, input, state)
    }
    #[inline(always)]
    fn parse_partial(
        &mut self,
        input: &mut Self::Input,
        state: &mut Self::PartialState,
    ) -> ConsumedResult<Self::Output, Self::Input> {
        self.parse_mode(PartialMode::default(), input, state)
    }
    #[inline]
    fn parse_mode_impl<M>(
        &mut self,
        mode: M,
        input: &mut Self::Input,
        state: &mut Self::PartialState,
    ) -> ConsumedResult<Value
        
        , I>
        where M: ParseMode
    {
        let JsonValue { .. } = *self;
        value_choice().parse_mode(mode, input, state)
    }
    
    #[inline]
    fn add_error(
        &mut self,
        errors: &mut Tracked<
            <I as StreamOnce>::Error
        >)
    {
        let JsonValue { .. } = *self;
        let mut parser = value_choice::<I>();
        parser.add_error(errors)
    }
    
    fn add_consumed_expected_error(
        &mut self,
        errors: &mut Tracked<
            <I as StreamOnce>::Error
        >)
    {
        let JsonValue { .. } = *self;
        let mut parser = value_choice::<I>();
        parser.add_consumed_expected_error(errors)
    }
}
#[inline(always)]
pub fn json_value_<I>() -> JsonValue<I>
    where <I as StreamOnce>::Error:
    ParseError<
        <I as StreamOnce>::Item,
        <I as StreamOnce>::Range,
        <I as StreamOnce>::Position
    >,
          I: SkipRangeStream<Item=u8, Range=BytesRange>
{
    JsonValue {
        __marker: PhantomData
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

#[test]
fn apache_incremental_test() {
    let data = include_str!("../../apache_builds.json");
    let mut parser = json_value();
    let mut state = Default::default();
    let full = Bytes::from_static(data.as_bytes());
    let mut remaining = data.len();
    while remaining > 0 {
        let start = full.len() - remaining;
        let slice = full.slice(start, start + remaining.min(86));
        let mut buf = BytesBuf::new(slice);
        println!("Parsing: {:?}", String::from_utf8_lossy(buf.next_bytes()));
    
        let result = decode(&mut parser, easy::Stream(buf), &mut state);
        println!("Partial: {:?}", result);
        match result {
            Ok((_, consumed)) => { remaining -= consumed },
            Err(e) => { panic!("ERR: {:?}", e)}
        }
//        remaining -= easy.0.pos();
//        match result {
//            ConsumedOk(r) => {},
//            EmptyOk(r) => {},
//            ConsumedErr(e) => { panic!("ConsumedErr: {:?}", e)},
//            EmptyErr(e) => { panic!("EmptyErr: {:?}", e)}
//        }
    }
    //println!("test: {:?}", result);
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
