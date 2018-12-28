#[feature(existential_type)]
use std::{
    fmt::{
        self,
        Display,
        Debug,
        Formatter
    }
};
use std::borrow::Cow;
use std::io;
use std::marker::PhantomData;

use bytes::{Buf, Bytes};
use combine::{
    ConsumedResult,
    easy::Stream,
    error::{
        FastResult,
        FastResult::*,
        Info,
        StreamError,
        Tracked,
        UnexpectedParse
    },
    ParseError,
    Parser,
    parser::function::parser,
    parser::ParseMode,
    Positioned,
    RangeStream,
    RangeStreamOnce,
    stream::{
        FullRangeStream,
        input_at_eof,
        PointerOffset,
        Range,
        Resetable,
        state::DefaultPositioned,
        state::IndexPositioner,
        StreamErrorFor,
        wrap_stream_error
    },
    StreamOnce,
};
use log::trace;

#[derive(Debug, PartialEq, Eq, Clone, Hash, Default)]
pub struct BytesRange(pub Bytes);

impl Display for BytesRange {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}
impl Range for BytesRange {
    #[inline]
    fn len(&self) -> usize {
        self.0.len()
    }
    
    #[inline]
    fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

pub trait SkipRangeStream: RangeStream {
    type SkipValue;
    fn skip_while<F>(&mut self, f: F) -> Result<Self::SkipValue, StreamErrorFor<Self>> where F: FnMut(Self::Item) -> bool;
    fn skip_range(&mut self, size: usize) -> Result<Self::SkipValue, StreamErrorFor<Self>>;
}

/// Removes items from the input while `predicate` returns `true`.
#[inline]
pub fn skip_stream_while<I, F>(input: &mut I, predicate: F) -> ConsumedResult<I::SkipValue, I>
    where
      F: FnMut(I::Item) -> bool,
      I: ?Sized + SkipRangeStream,
{
    let pos = input.checkpoint();
    match input.skip_while(predicate) {
        Err(err) => wrap_stream_error(input, err),
        Ok(x) => {
            if input.is_partial() && input_at_eof(input) {
                // Partial inputs which encounter end of file must fail to let more input be
                // retrieved
                ConsumedErr(I::Error::from_error(
                    input.position(),
                    StreamError::end_of_input(),
                ))
            } else if input.distance(&pos) == 0 {
                EmptyOk(x)
            } else {
                ConsumedOk(x)
            }
        }
    }
}

pub struct SkipWhile<I, F> {
    f: F,
    p: PhantomData<I>
}

impl<I, F> SkipWhile<I, F> where I: SkipRangeStream, F: FnMut(I::Item) -> bool {

}

pub fn skip_while<I, F>(f: F) -> SkipWhile<I, F> where I: SkipRangeStream, F: FnMut(I::Item) -> bool {
    SkipWhile { f, p: Default::default() }
}

pub fn skip_while1<I, F>(mut f: F) -> impl Parser<Input=I, Output=I::SkipValue, PartialState = ()> where I: SkipRangeStream, F: FnMut(I::Item) -> bool {
    parser(move |i: &mut I| {
        let x = i.checkpoint();
        match skip_while(&mut f).parse_stream_consumed(i) {
            ConsumedOk(x) => ConsumedOk(x),
            EmptyOk(_) => EmptyErr(Tracked::from(I::Error::from_error(
                i.position(),
                StreamError::end_of_input(),
            ))),
            EmptyErr(e) => EmptyErr(e),
            ConsumedErr(e) => ConsumedErr(e)
        }.into()
    })
}
impl<I, F> Parser for SkipWhile<I, F>
    where
      I: SkipRangeStream,
      F: FnMut(I::Item) -> bool,
{
    type Input = I;
    type Output = I::SkipValue;
    type PartialState = usize;
    
    parse_mode!();
    #[inline]
    fn parse_mode_impl<M>(
        &mut self,
        mode: M,
        input: &mut Self::Input,
        state: &mut Self::PartialState,
    ) -> ConsumedResult<Self::Output, Self::Input>
        where
          M: ParseMode,
    {
        let before = input.checkpoint();
    
        if !input.is_partial() {
            skip_stream_while(input, &mut self.f)
        } else if mode.is_first() || *state == 0 {
            let result = skip_stream_while(input, &mut self.f);
            if let ConsumedErr(_) = result {
                *state = input.distance(&before);
                input.reset(before);
            }
            result
        } else {
            if input.skip_range(*state).is_err() {
                panic!("recognize errored when restoring the input stream to its expected state");
            }
    
            let r = match skip_stream_while(input, &mut self.f) {
                ConsumedOk(r) => {
                    *state = 0;
                    ConsumedOk(r)
                },
                EmptyOk(r) => {
                    *state = 0;
                    EmptyOk(r)
                },
                EmptyErr(err) => return EmptyErr(err),
                ConsumedErr(err) => {
                    *state = input.distance(&before);
                    input.reset(before);
                    return ConsumedErr(err);
                }
            };
            r
        }
    }
}


fn printbuf(mut buf: &[u8]) -> Cow<str> {
    let b = if buf.len() > 10{
        &buf[..10]
    } else {
        buf
    };
    String::from_utf8_lossy(b)
}

#[derive(Debug, Clone)]
pub struct BytesBuf {
    cur: io::Cursor<Bytes>
}


impl BytesBuf {
    pub fn new(b: Bytes) -> Self {
        BytesBuf { cur: io::Cursor::new(b) }
    }
    
    #[inline]
    pub fn slice_while<F>(&self, f: F) -> Result<&[u8], StreamErrorFor<Self>> where F: FnMut(u8) -> bool {
        let mut slice = self.next_bytes();
        let result = slice.uncons_while(f);
        println!("Bytes slice while: {:?}", result.map(|b| String::from_utf8_lossy(b)));
        result
    }
    
    pub fn pos(&self) -> usize{
        self.cur.position() as usize
    }
    
    #[inline(always)]
    pub fn next_bytes(&self) -> &[u8] {
        let pos = self.cur.position() as usize;
        &self.cur.get_ref().as_ref()[pos..]
    }
    
    fn buf(&self) -> &Bytes {
        self.cur.get_ref()
    }
}

impl Positioned for BytesBuf {
    #[inline(always)]
    fn position(&self) -> Self::Position {
        self.cur.position() as usize
    }
}

impl DefaultPositioned for BytesBuf {
    type Positioner = IndexPositioner;
}

impl StreamOnce for BytesBuf {
    type Item = u8;
    type Range = BytesRange;
    type Position = usize;
    type Error = UnexpectedParse;
    
    #[inline]
    fn uncons(&mut self) -> Result<u8, StreamErrorFor<Self>> {
        let slice = self.cur.get_ref().as_ref();
        let next = slice.get(self.cur.position() as usize);
        println!("Bytes uncons: {:?}", next.map(|s| (char::from(*s), printbuf(self.cur.bytes()))));
        let next = match next {
            Some(f) => *f,
            None => return Err(UnexpectedParse::Eoi)
        };
        self.cur.advance(1);
        Ok(next)
    }
}

impl SkipRangeStream for BytesBuf {
    type SkipValue = ();
    
    #[inline]
    fn skip_while<F>(&mut self, f: F) -> Result<(), StreamErrorFor<Self>> where F: FnMut(Self::Item) -> bool {
        let len = self.slice_while(f)?.len();
        self.cur.advance(len);
        Ok(())
    }
    
    #[inline]
    fn skip_range(&mut self, size: usize) -> Result<(), StreamErrorFor<Self>> {
        let result = if size < self.cur.remaining() {
            let position = self.cur.position();
            self.cur.set_position(position + size as u64);
            Ok(())
        } else {
            Err(UnexpectedParse::Eoi)
        };
        println!("Bytes skip range {:?} : {:?}", size, result);
        result
    }
}

impl<I> SkipRangeStream for Stream<I> where I: SkipRangeStream {
    type SkipValue = I::SkipValue;
    
    #[inline]
    fn skip_while<F>(&mut self, f: F) -> Result<I::SkipValue, StreamErrorFor<Self>> where F: FnMut(I::Item) -> bool {
        self.0.skip_while(f).map_err(StreamError::into_other)
    }
    
    #[inline]
    fn skip_range(&mut self, size: usize) -> Result<I::SkipValue, StreamErrorFor<Self>> {
        self.0.skip_range(size).map_err(StreamError::into_other)
    }
}

impl Resetable for BytesBuf {
    type Checkpoint = u64;
    
    #[inline]
    fn checkpoint(&self) -> Self::Checkpoint {
        println!("Checkpoint: {} {:?}", self.cur.position(), printbuf(&*self.cur.bytes()));
        self.cur.position()
    }
    
    #[inline]
    fn reset(&mut self, checkpoint: Self::Checkpoint) {
        println!("Reset: {} -> {} {:?}", self.cur.position(), checkpoint, printbuf(&*self.cur.bytes()));
        self.cur.set_position(checkpoint)
    }
}

impl RangeStreamOnce for BytesBuf {
    #[inline]
    fn uncons_range(&mut self, size: usize) -> Result<BytesRange, StreamErrorFor<Self>> {
        let result = if size < self.cur.remaining() {
            let position = self.cur.position() as usize;
            let bytes = self.cur.get_ref().slice(position, position + size);
            self.cur.advance(size);
            Ok(BytesRange(bytes))
        } else {
            Err(UnexpectedParse::Eoi)
        };
        println!("Bytes uncons_range {:?} : {:?}", size, result.as_ref().map(|b| String::from_utf8_lossy(&*b.0)));
        result
    }
    
    #[inline]
    fn uncons_while<F>(&mut self, mut f: F) -> Result<BytesRange, StreamErrorFor<Self>> where
      F: FnMut(Self::Item) -> bool {
        println!("Uncons while");
        let result = self.slice_while(&mut f);
        match result {
            Ok(s) if s.len() > 0 => {
                let bytes = self.buf().slice_ref (s);
                self.cur.advance(s.len());
                Ok(BytesRange(bytes))
            },
            Ok(_) => Ok(BytesRange(Bytes::new())),
            Err(e) => Err(e)
        }
    }
    
    #[inline]
    fn uncons_while1<F>(&mut self, mut f: F) -> FastResult<Self::Range, StreamErrorFor<Self>>
        where
          F: FnMut(Self::Item) -> bool,
    {
        use self::FastResult::*;
        let mut slice = &*self.cur.bytes();
        let result = slice.uncons_while1(f);
        println!("Bytes uncons_while1: {:?}", result.map(|b| printbuf(b)));
        match result {
            ConsumedOk(s) => {
                let bytes = self.cur.get_ref().slice_ref(s);
                self.cur.advance(s.len());
                ConsumedOk(BytesRange(bytes))
            },
            EmptyErr(e) => EmptyErr(e),
            _ => unreachable!()
        }
        
    }
    
    fn distance(&self, end: &u64) -> usize {
        let dist = self.cur.position() as usize - *end as usize;
        println!("Bytes distance {:?} -> {:?} res: {:?} {:?}", self.cur.position(), end, dist, printbuf(&*self.cur.bytes()));
        dist
    }
}


