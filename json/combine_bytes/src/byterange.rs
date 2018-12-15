use bytes::Bytes;
use log::trace;
use combine::{
    error::{
        UnexpectedParse,
        FastResult
    },
    RangeStream,
    RangeStreamOnce,
    StreamOnce,
    Positioned,
    ParseError,
    stream::{
        Resetable,
        Range,
        PointerOffset,
        StreamErrorFor
    }
};
use std::{
    fmt::{
        self,
        Display,
        Debug,
        Formatter
    }
};
use combine::stream::state::DefaultPositioned;
use combine::stream::state::IndexPositioner;

#[derive(Debug, PartialEq, Eq, Clone, Hash)]
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

impl Positioned for BytesRange {
    #[inline(always)]
    fn position(&self) -> Self::Position {
        PointerOffset(self.0.as_ptr() as usize)
    }
}

impl DefaultPositioned for BytesRange {
    type Positioner = IndexPositioner;
}

impl StreamOnce for BytesRange {
    type Item = u8;
    type Range = BytesRange;
    type Position = PointerOffset;
    type Error = UnexpectedParse;
    
    #[inline]
    fn uncons(&mut self) -> Result<u8, StreamErrorFor<Self>> {
        let next = self.0.split_first();
        println!("Bytes uncons: {:?}", next.map(|(s,v)| (char::from(*s), String::from_utf8_lossy(v))));
        let next = match next {
            Some((f, _)) => *f,
            None => return Err(UnexpectedParse::Eoi)
        };
        self.0.advance(1);
        Ok(next)
    }
}

impl Resetable for BytesRange {
    type Checkpoint = Self;
    
    fn checkpoint(&self) -> Self::Checkpoint {
        self.clone()
    }
    
    fn reset(&mut self, checkpoint: Self::Checkpoint) {
        *self = checkpoint;
    }
}

impl RangeStreamOnce for BytesRange {
    #[inline]
    fn uncons_range(&mut self, size: usize) -> Result<Self, StreamErrorFor<Self>> {
        let result = if size < self.0.len() {
            let bytes = self.0.split_to(size);
            Ok(BytesRange(bytes))
        } else {
            Err(UnexpectedParse::Eoi)
        };
        println!("Bytes uncons_range {:?} : {:?}", size, result.as_ref().map(|b| String::from_utf8_lossy(&*b.0)));
        result
    }
    
    #[inline]
    fn uncons_while<F>(&mut self, f: F) -> Result<Self, StreamErrorFor<Self>> where
      F: FnMut(Self::Item) -> bool {
        let mut slice = self.0.as_ref();
        let result = slice.uncons_while(f);
        println!("Bytes uncons_while: {:?}", result.map(|b| String::from_utf8_lossy(b)));
        match result {
            Ok(s) => {
                let bytes = self.0.split_to(s.len());
                Ok(BytesRange(bytes))
            }
            Err(e) => Err(e)
        }
    }
    
    #[inline]
    fn uncons_while1<F>(&mut self, mut f: F) -> FastResult<Self::Range, StreamErrorFor<Self>>
        where
          F: FnMut(Self::Item) -> bool,
    {
        use self::FastResult::*;
        let mut slice = self.0.as_ref();
        let result = slice.uncons_while1(f);
        println!("Bytes uncons_while1: {:?}", result.map(|b| String::from_utf8_lossy(b)));
        match result {
            ConsumedOk(s) => {
                let bytes = self.0.split_to(s.len());
                ConsumedOk(BytesRange(bytes))
            },
            EmptyErr(e) => EmptyErr(e),
            _ => unreachable!()
        }
        
    }
    
    fn distance(&self, end: &Self) -> usize {
        end.len() - self.len()
    }
}


