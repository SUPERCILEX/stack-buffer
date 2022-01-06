use std::{
    cmp, fmt, io,
    io::{BufRead, IoSliceMut, Read, ReadBuf, Seek, SeekFrom},
    mem::MaybeUninit,
    str::from_utf8,
};

/// The `StackBufReader<N, R>` struct adds buffering to any reader.
///
/// See [`BufReader`][std::io::BufReader] for more details.
///
/// # Examples
///
/// ```no_run
/// use std::io::prelude::*;
/// use std::fs::File;
/// use stack_buffer::StackBufReader;
///
/// fn main() -> std::io::Result<()> {
///     let f = File::open("log.txt")?;
///     let mut reader = StackBufReader::<_, 4096>::new(f);
///
///     let mut line = String::new();
///     let len = reader.read_line(&mut line)?;
///     println!("First line is {} bytes long", len);
///     Ok(())
/// }
/// ```
pub struct StackBufReader<R, const N: usize = 4096> {
    inner: R,
    buf: [MaybeUninit<u8>; N],
    pos: usize,
    cap: usize,
    init: usize,
}

impl<R: Read, const N: usize> StackBufReader<R, N> {
    /// Creates a new `StackBufReader<R, N>`.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use std::fs::File;
    /// use stack_buffer::StackBufReader;
    ///
    /// fn main() -> std::io::Result<()> {
    ///     let f = File::open("log.txt")?;
    ///     let reader = StackBufReader::<_, 4096>::new(f);
    ///     Ok(())
    /// }
    /// ```
    pub fn new(inner: R) -> StackBufReader<R, N> {
        StackBufReader {
            inner,
            buf: unsafe { MaybeUninit::uninit().assume_init() },
            pos: 0,
            cap: 0,
            init: 0,
        }
    }
}

impl<R, const N: usize> StackBufReader<R, N> {
    /// Gets a reference to the underlying reader.
    ///
    /// It is inadvisable to directly read from the underlying reader.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use std::fs::File;
    /// use stack_buffer::StackBufReader;
    ///
    /// fn main() -> std::io::Result<()> {
    ///     let f1 = File::open("log.txt")?;
    ///     let reader = StackBufReader::<_, 4096>::new(f1);
    ///
    ///     let f2 = reader.get_ref();
    ///     Ok(())
    /// }
    /// ```
    pub fn get_ref(&self) -> &R {
        &self.inner
    }

    /// Gets a mutable reference to the underlying reader.
    ///
    /// It is inadvisable to directly read from the underlying reader.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use std::fs::File;
    /// use stack_buffer::StackBufReader;
    ///
    /// fn main() -> std::io::Result<()> {
    ///     let f1 = File::open("log.txt")?;
    ///     let mut reader = StackBufReader::<_, 4096>::new(f1);
    ///
    ///     let f2 = reader.get_mut();
    ///     Ok(())
    /// }
    /// ```
    pub fn get_mut(&mut self) -> &mut R {
        &mut self.inner
    }

    /// Returns a reference to the internally buffered data.
    ///
    /// Unlike [`fill_buf`], this will not attempt to fill the buffer if it is empty.
    ///
    /// [`fill_buf`]: BufRead::fill_buf
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use std::io::BufRead;
    /// use std::fs::File;
    /// use stack_buffer::StackBufReader;
    ///
    /// fn main() -> std::io::Result<()> {
    ///     let f = File::open("log.txt")?;
    ///     let mut reader = StackBufReader::<_, 4096>::new(f);
    ///     assert!(reader.buffer().is_empty());
    ///
    ///     if reader.fill_buf()?.len() > 0 {
    ///         assert!(!reader.buffer().is_empty());
    ///     }
    ///     Ok(())
    /// }
    /// ```
    pub fn buffer(&self) -> &[u8] {
        // SAFETY: self.cap is always <= self.init, so self.buf[self.pos..self.cap] is always init
        unsafe { MaybeUninit::slice_assume_init_ref(&self.buf[self.pos..self.cap]) }
    }

    /// Returns the number of bytes the internal buffer can hold at once.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use std::io::BufRead;
    /// use std::fs::File;
    /// use stack_buffer::StackBufReader;
    ///
    /// fn main() -> std::io::Result<()> {
    ///     let f = File::open("log.txt")?;
    ///     let mut reader = StackBufReader::<_, 4096>::new(f);
    ///
    ///     let capacity = reader.capacity();
    ///     let buffer = reader.fill_buf()?;
    ///     assert!(buffer.len() <= capacity);
    ///     Ok(())
    /// }
    /// ```
    pub fn capacity(&self) -> usize {
        self.buf.len()
    }

    /// Unwraps this `BufReader<R>`, returning the underlying reader.
    ///
    /// Note that any leftover data in the internal buffer is lost. Therefore,
    /// a following read from the underlying reader may lead to data loss.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use std::fs::File;
    /// use stack_buffer::StackBufReader;
    ///
    /// fn main() -> std::io::Result<()> {
    ///     let f1 = File::open("log.txt")?;
    ///     let reader = StackBufReader::<_, 4096>::new(f1);
    ///
    ///     let f2 = reader.into_inner();
    ///     Ok(())
    /// }
    /// ```
    pub fn into_inner(self) -> R {
        self.inner
    }

    /// Invalidates all data in the internal buffer.
    #[inline]
    fn discard_buffer(&mut self) {
        self.pos = 0;
        self.cap = 0;
    }
}

impl<R: Seek, const N: usize> StackBufReader<R, N> {
    /// Seeks relative to the current position. If the new position lies within the buffer,
    /// the buffer will not be flushed, allowing for more efficient seeks.
    /// This method does not return the location of the underlying reader, so the caller
    /// must track this information themselves if it is required.
    pub fn seek_relative(&mut self, offset: i64) -> io::Result<()> {
        let pos = self.pos as u64;
        if offset < 0 {
            if let Some(new_pos) = pos.checked_sub((-offset) as u64) {
                self.pos = new_pos as usize;
                return Ok(());
            }
        } else if let Some(new_pos) = pos.checked_add(offset as u64) {
            if new_pos <= self.cap as u64 {
                self.pos = new_pos as usize;
                return Ok(());
            }
        }

        self.seek(SeekFrom::Current(offset)).map(drop)
    }
}

impl<R: Read, const N: usize> Read for StackBufReader<R, N> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        // If we don't have any buffered data and we're doing a massive read
        // (larger than our internal buffer), bypass our internal buffer
        // entirely.
        if self.pos == self.cap && buf.len() >= self.buf.len() {
            self.discard_buffer();
            return self.inner.read(buf);
        }
        let nread = {
            let mut rem = self.fill_buf()?;
            rem.read(buf)?
        };
        self.consume(nread);
        Ok(nread)
    }

    fn read_vectored(&mut self, bufs: &mut [IoSliceMut<'_>]) -> io::Result<usize> {
        let total_len = bufs.iter().map(|b| b.len()).sum::<usize>();
        if self.pos == self.cap && total_len >= self.buf.len() {
            self.discard_buffer();
            return self.inner.read_vectored(bufs);
        }
        let nread = {
            let mut rem = self.fill_buf()?;
            rem.read_vectored(bufs)?
        };
        self.consume(nread);
        Ok(nread)
    }

    fn is_read_vectored(&self) -> bool {
        self.inner.is_read_vectored()
    }

    // The inner reader might have an optimized `read_to_end`. Drain our buffer and then
    // delegate to the inner implementation.
    fn read_to_end(&mut self, buf: &mut Vec<u8>) -> io::Result<usize> {
        let nread = self.cap - self.pos;
        buf.extend_from_slice(self.buffer());
        self.discard_buffer();
        Ok(nread + self.inner.read_to_end(buf)?)
    }

    // The inner reader might have an optimized `read_to_end`. Drain our buffer and then
    // delegate to the inner implementation.
    fn read_to_string(&mut self, buf: &mut String) -> io::Result<usize> {
        // In the general `else` case below we must read bytes into a side buffer, check
        // that they are valid UTF-8, and then append them to `buf`. This requires a
        // potentially large memcpy.
        //
        // If `buf` is empty--the most common case--we can leverage `append_to_string`
        // to read directly into `buf`'s internal byte buffer, saving an allocation and
        // a memcpy.
        if buf.is_empty() {
            // `append_to_string`'s safety relies on the buffer only being appended to since
            // it only checks the UTF-8 validity of new data. If there were existing content in
            // `buf` then an untrustworthy reader (i.e. `self.inner`) could not only append
            // bytes but also modify existing bytes and render them invalid. On the other hand,
            // if `buf` is empty then by definition any writes must be appends and
            // `append_to_string` will validate all of the new bytes.
            unsafe { append_to_string(buf, |b| self.read_to_end(b)) }
        } else {
            // We cannot append our byte buffer directly onto the `buf` String as there could
            // be an incomplete UTF-8 sequence that has only been partially read. We must read
            // everything into a side buffer first and then call `from_utf8` on the complete
            // buffer.
            let mut bytes = Vec::new();
            self.read_to_end(&mut bytes)?;
            let string = from_utf8(&bytes).map_err(|_| {
                io::Error::new(
                    io::ErrorKind::InvalidData,
                    "stream did not contain valid UTF-8",
                )
            })?;
            *buf += string;
            Ok(string.len())
        }
    }

    // Small read_exacts from a BufReader are extremely common when used with a deserializer.
    // The default implementation calls read in a loop, which results in surprisingly poor code
    // generation for the common path where the buffer has enough bytes to fill the passed-in
    // buffer.
    fn read_exact(&mut self, buf: &mut [u8]) -> io::Result<()> {
        if self.buffer().len() >= buf.len() {
            buf.copy_from_slice(&self.buffer()[..buf.len()]);
            self.consume(buf.len());
            return Ok(());
        }

        default_read_exact(self, buf)
    }

    fn read_buf(&mut self, buf: &mut ReadBuf<'_>) -> io::Result<()> {
        // If we don't have any buffered data and we're doing a massive read
        // (larger than our internal buffer), bypass our internal buffer
        // entirely.
        if self.pos == self.cap && buf.remaining() >= self.buf.len() {
            self.discard_buffer();
            return self.inner.read_buf(buf);
        }

        let prev = buf.filled_len();

        let mut rem = self.fill_buf()?;
        rem.read_buf(buf)?;

        self.consume(buf.filled_len() - prev); //slice impl of read_buf known to never unfill buf

        Ok(())
    }
}

struct Guard<'a> {
    buf: &'a mut Vec<u8>,
    len: usize,
}

impl Drop for Guard<'_> {
    fn drop(&mut self) {
        unsafe {
            self.buf.set_len(self.len);
        }
    }
}

unsafe fn append_to_string<F>(buf: &mut String, f: F) -> io::Result<usize>
where
    F: FnOnce(&mut Vec<u8>) -> io::Result<usize>,
{
    let mut g = Guard {
        len: buf.len(),
        buf: buf.as_mut_vec(),
    };
    let ret = f(g.buf);
    if from_utf8(&g.buf[g.len..]).is_err() {
        ret.and_then(|_| {
            Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "stream did not contain valid UTF-8",
            ))
        })
    } else {
        g.len = g.buf.len();
        ret
    }
}

fn default_read_exact<R: Read + ?Sized>(this: &mut R, mut buf: &mut [u8]) -> io::Result<()> {
    while !buf.is_empty() {
        match this.read(buf) {
            Ok(0) => break,
            Ok(n) => {
                let tmp = buf;
                buf = &mut tmp[n..];
            }
            Err(ref e) if e.kind() == io::ErrorKind::Interrupted => {}
            Err(e) => return Err(e),
        }
    }
    if !buf.is_empty() {
        Err(io::Error::new(
            io::ErrorKind::UnexpectedEof,
            "failed to fill whole buffer",
        ))
    } else {
        Ok(())
    }
}

impl<R: Read, const N: usize> BufRead for StackBufReader<R, N> {
    fn fill_buf(&mut self) -> io::Result<&[u8]> {
        // If we've reached the end of our internal buffer then we need to fetch
        // some more data from the underlying reader.
        // Branch using `>=` instead of the more correct `==`
        // to tell the compiler that the pos..cap slice is always valid.
        if self.pos >= self.cap {
            debug_assert!(self.pos == self.cap);

            let mut readbuf = ReadBuf::uninit(&mut self.buf);

            // SAFETY: `self.init` is either 0 or set to `readbuf.initialized_len()`
            // from the last time this function was called
            unsafe {
                readbuf.assume_init(self.init);
            }

            self.inner.read_buf(&mut readbuf)?;

            self.cap = readbuf.filled_len();
            self.init = readbuf.initialized_len();

            self.pos = 0;
        }
        Ok(self.buffer())
    }

    fn consume(&mut self, amt: usize) {
        self.pos = cmp::min(self.pos + amt, self.cap);
    }
}

impl<R, const N: usize> fmt::Debug for StackBufReader<R, N>
where
    R: fmt::Debug,
{
    fn fmt(&self, fmt: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt.debug_struct("BufReader")
            .field("reader", &self.inner)
            .field(
                "buffer",
                &format_args!("{}/{}", self.cap - self.pos, self.buf.len()),
            )
            .finish()
    }
}

impl<R: Seek, const N: usize> Seek for StackBufReader<R, N> {
    /// Seek to an offset, in bytes, in the underlying reader.
    ///
    /// See [`BufReader::seek`][std::io::BufReader::seek] for more details.
    fn seek(&mut self, pos: SeekFrom) -> io::Result<u64> {
        let result: u64;
        if let SeekFrom::Current(n) = pos {
            let remainder = (self.cap - self.pos) as i64;
            // it should be safe to assume that remainder fits within an i64 as the alternative
            // means we managed to allocate 8 exbibytes and that's absurd.
            // But it's not out of the realm of possibility for some weird underlying reader to
            // support seeking by i64::MIN so we need to handle underflow when subtracting
            // remainder.
            if let Some(offset) = n.checked_sub(remainder) {
                result = self.inner.seek(SeekFrom::Current(offset))?;
            } else {
                // seek backwards by our remainder, and then by the offset
                self.inner.seek(SeekFrom::Current(-remainder))?;
                self.discard_buffer();
                result = self.inner.seek(SeekFrom::Current(n))?;
            }
        } else {
            // Seeking with Start/End doesn't care about our buffer length.
            result = self.inner.seek(pos)?;
        }
        self.discard_buffer();
        Ok(result)
    }

    /// Returns the current seek position from the start of the stream.
    ///
    /// See [`BufReader::stream_position`][std::io::BufReader::stream_position] for more details.
    ///
    /// # Example
    ///
    /// ```no_run
    /// use std::{
    ///     io::{self, BufRead, Seek},
    ///     fs::File,
    /// };
    /// use stack_buffer::StackBufReader;
    ///
    /// fn main() -> io::Result<()> {
    ///     let mut f = StackBufReader::<_, 4096>::new(File::open("foo.txt")?);
    ///
    ///     let before = f.stream_position()?;
    ///     f.read_line(&mut String::new())?;
    ///     let after = f.stream_position()?;
    ///
    ///     println!("The first line was {} bytes long", after - before);
    ///     Ok(())
    /// }
    /// ```
    fn stream_position(&mut self) -> io::Result<u64> {
        let remainder = (self.cap - self.pos) as u64;
        self.inner.stream_position().map(|pos| {
            pos.checked_sub(remainder).expect(
                "overflow when subtracting remaining buffer size from inner stream position",
            )
        })
    }
}
