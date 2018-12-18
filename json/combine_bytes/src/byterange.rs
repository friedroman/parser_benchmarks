use std::{
    fmt::{
        self,
        Display,
        Debug,
        Formatter
    }
};
use std::io;

use bytes::{Buf, Bytes};
use combine::{
    error::{
        FastResult,
        UnexpectedParse
    },
    ParseError,
    Positioned,
    RangeStream,
    RangeStreamOnce,
    stream::{
        PointerOffset,
        Range,
        Resetable,
        StreamErrorFor
    },
    StreamOnce
};
use combine::stream::state::DefaultPositioned;
use combine::stream::state::IndexPositioner;
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

#[derive(Debug, Clone)]
pub struct BytesBuf {
    cur: io::Cursor<Bytes>
}

impl BytesBuf {
    pub fn new(b: Bytes) -> Self {
        BytesBuf { cur: io::Cursor::new(b) }
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
        let next = self.cur.bytes().first();
        //println!("Bytes uncons: {:?}", next.map(|s| (char::from(*s), String::from_utf8_lossy(self.cur.bytes()))));
        let next = match next {
            Some(f) => *f,
            None => return Err(UnexpectedParse::Eoi)
        };
        self.cur.advance(1);
        Ok(next)
    }
}

impl Resetable for BytesBuf {
    type Checkpoint = u64;
    
    fn checkpoint(&self) -> Self::Checkpoint {
        //println!("Checkpoint: {} {:?}", self.cur.position(), String::from_utf8_lossy(&*self.cur.bytes()));
        self.cur.position()
    }
    
    fn reset(&mut self, checkpoint: Self::Checkpoint) {
        //println!("Reset: {} {} {:?}", checkpoint, self.cur.position(), String::from_utf8_lossy(&*self.cur.bytes()));
        self.cur.set_position(checkpoint)
    }
}

impl RangeStreamOnce for BytesBuf {
    #[inline]
    fn uncons_range(&mut self, size: usize) -> Result<BytesRange, StreamErrorFor<Self>> {
        //println!("Uncons_range {:?} : {:?}", size, String::from_utf8_lossy(&**self.cur.get_ref()));
        let result = if size < self.cur.remaining() {
            let position = self.cur.position() as usize;
            let bytes = self.cur.get_ref().slice(position, position + size);
            self.cur.advance(size);
            Ok(BytesRange(bytes))
        } else {
            Err(UnexpectedParse::Eoi)
        };
        //println!("Bytes uncons_range {:?} : {:?}", size, result.as_ref().map(|b| String::from_utf8_lossy(&*b.0)));
        result
    }
    
    #[inline]
    fn uncons_while<F>(&mut self, f: F) -> Result<BytesRange, StreamErrorFor<Self>> where
      F: FnMut(Self::Item) -> bool {
        let mut slice = self.cur.bytes();
        let result = slice.uncons_while(f);
        //println!("Bytes uncons_while: {:?}", result.map(|b| String::from_utf8_lossy(b)));
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
        //println!("Bytes uncons_while1: {:?}", result.map(|b| String::from_utf8_lossy(b)));
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
        //println!("Bytes distance1 {:?} : {:?} res: {:?} {:?}", self.cur.position(), end, dist, String::from_utf8_lossy(&*self.cur.bytes()));
        dist
    }
}


