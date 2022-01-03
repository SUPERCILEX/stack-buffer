use std::{
    fmt, io,
    io::{Error, ErrorKind, IoSlice, Seek, SeekFrom, Write},
    mem::MaybeUninit,
    ptr,
};

/// Wraps a writer and buffers its output.
///
/// See [`BufWriter`][std::io::BufWriter] for more details.
///
/// # Examples
///
/// Let's write the numbers one through ten to a [`TcpStream`]:
///
/// ```no_run
/// use std::io::prelude::*;
/// use std::net::TcpStream;
///
/// let mut stream = TcpStream::connect("127.0.0.1:34254").unwrap();
///
/// for i in 0..10 {
///     stream.write(&[i+1]).unwrap();
/// }
/// ```
///
/// Because we're not buffering, we write each one in turn, incurring the
/// overhead of a system call per byte written. We can fix this with a
/// `StackBufWriter<W, N>`:
///
/// ```no_run
/// use std::io::prelude::*;
/// use std::net::TcpStream;
/// use stack_buffer::StackBufWriter;
///
/// let mut stream = StackBufWriter::<_, 4096>::new(TcpStream::connect("127.0.0.1:34254").unwrap());
///
/// for i in 0..10 {
///     stream.write(&[i+1]).unwrap();
/// }
/// stream.flush().unwrap();
/// ```
///
/// By wrapping the stream with a `StackBufWriter<W, N>`, these ten writes are all grouped
/// together by the buffer and will all be written out in one system call when
/// the `stream` is flushed.
///
/// [`TcpStream`]: crate::net::TcpStream
pub struct StackBufWriter<W: Write, const N: usize> {
    inner: W,
    buf: [MaybeUninit<u8>; N],
    start: usize,
    end: usize,
    // #30888: If the inner writer panics in a call to write, we don't want to
    // write the buffered data a second time in BufWriter's destructor. This
    // flag tells the Drop impl if it should skip the flush.
    panicked: bool,
}

impl<W: Write, const N: usize> StackBufWriter<W, N> {
    /// Creates a new `StackBufWriter<W, N>`.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use std::net::TcpStream;
    /// use stack_buffer::StackBufWriter;
    ///
    /// let mut buffer = StackBufWriter::<_, 4096>::new(TcpStream::connect("127.0.0.1:34254").unwrap());
    /// ```
    pub fn new(inner: W) -> StackBufWriter<W, N> {
        StackBufWriter {
            inner,
            buf: unsafe { MaybeUninit::uninit().assume_init() },
            start: 0,
            end: 0,
            panicked: false,
        }
    }

    /// Send data in our local buffer into the inner writer, looping as
    /// necessary until either it's all been sent or an error occurs.
    ///
    /// Because all the data in the buffer has been reported to our owner as
    /// "successfully written" (by returning nonzero success values from
    /// `write`), any 0-length writes from `inner` must be reported as i/o
    /// errors from this method.
    fn flush_buf(&mut self) -> io::Result<()> {
        /// Helper struct to ensure the buffer is updated after all the writes
        /// are complete. It tracks the number of written bytes and drains them
        /// all from the front of the buffer when dropped.
        struct BufGuard<'a> {
            buffer: &'a [u8],
            start: &'a mut usize,
            end: &'a mut usize,

            written: usize,
        }

        impl<'a> BufGuard<'a> {
            fn new(buffer: &'a [u8], start: &'a mut usize, end: &'a mut usize) -> Self {
                Self {
                    buffer,
                    start,
                    end,
                    written: 0,
                }
            }

            /// The unwritten part of the buffer
            fn remaining(&self) -> &[u8] {
                &self.buffer[self.written..]
            }

            /// Flag some bytes as removed from the front of the buffer
            fn consume(&mut self, amt: usize) {
                self.written += amt;
            }

            /// true if all of the bytes have been written
            fn done(&self) -> bool {
                self.written >= self.buffer.len()
            }
        }

        impl Drop for BufGuard<'_> {
            fn drop(&mut self) {
                *self.start += self.written;
                if self.start >= self.end {
                    debug_assert_eq!(self.start, self.end);
                    *self.start = 0;
                    *self.end = 0;
                }
            }
        }

        let mut guard = BufGuard::new(
            unsafe { MaybeUninit::slice_assume_init_ref(&self.buf[self.start..self.end]) },
            &mut self.start,
            &mut self.end,
        );
        while !guard.done() {
            self.panicked = true;
            let r = self.inner.write(guard.remaining());
            self.panicked = false;

            match r {
                Ok(0) => {
                    return Err(Error::new(
                        ErrorKind::WriteZero,
                        "failed to write the buffered data",
                    ));
                }
                Ok(n) => guard.consume(n),
                Err(ref e) if e.kind() == io::ErrorKind::Interrupted => {}
                Err(e) => return Err(e),
            }
        }
        Ok(())
    }

    /// Gets a reference to the underlying writer.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use std::net::TcpStream;
    /// use stack_buffer::StackBufWriter;
    ///
    /// let mut buffer = StackBufWriter::<_, 4096>::new(TcpStream::connect("127.0.0.1:34254").unwrap());
    ///
    /// // we can use reference just like buffer
    /// let reference = buffer.get_ref();
    /// ```
    pub fn get_ref(&self) -> &W {
        &self.inner
    }

    /// Gets a mutable reference to the underlying writer.
    ///
    /// It is inadvisable to directly write to the underlying writer.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use std::net::TcpStream;
    /// use stack_buffer::StackBufWriter;
    ///
    /// let mut buffer = StackBufWriter::<_, 4096>::new(TcpStream::connect("127.0.0.1:34254").unwrap());
    ///
    /// // we can use reference just like buffer
    /// let reference = buffer.get_mut();
    /// ```
    pub fn get_mut(&mut self) -> &mut W {
        &mut self.inner
    }

    /// Returns a reference to the internally buffered data.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use std::net::TcpStream;
    /// use stack_buffer::StackBufWriter;
    ///
    /// let buf_writer = StackBufWriter::<_, 4096>::new(TcpStream::connect("127.0.0.1:34254").unwrap());
    ///
    /// // See how many bytes are currently buffered
    /// let bytes_buffered = buf_writer.buffer().len();
    /// ```
    pub fn buffer(&self) -> &[u8] {
        unsafe { MaybeUninit::slice_assume_init_ref(&self.buf[self.start..self.end]) }
    }

    /// Returns the number of bytes the internal buffer can hold without flushing.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use std::net::TcpStream;
    /// use stack_buffer::StackBufWriter;
    ///
    /// let buf_writer = StackBufWriter::<_, 4096>::new(TcpStream::connect("127.0.0.1:34254").unwrap());
    ///
    /// // Check the capacity of the inner buffer
    /// let capacity = buf_writer.capacity();
    /// // Calculate how many bytes can be written without flushing
    /// let without_flush = capacity - buf_writer.buffer().len();
    /// ```
    pub fn capacity(&self) -> usize {
        self.buf.len()
    }

    // Ensure this function does not get inlined into `write`, so that it
    // remains inlineable and its common path remains as short as possible.
    // If this function ends up being called frequently relative to `write`,
    // it's likely a sign that the client is using an improperly sized buffer
    // or their write patterns are somewhat pathological.
    #[cold]
    #[inline(never)]
    fn write_cold(&mut self, buf: &[u8]) -> io::Result<usize> {
        if buf.len() > self.spare_capacity() {
            self.flush_buf()?;
        }

        // Why not len > capacity? To avoid a needless trip through the buffer when the input
        // exactly fills it. We'd just need to flush it to the underlying writer anyway.
        if buf.len() >= self.buf.len() {
            self.panicked = true;
            let r = self.get_mut().write(buf);
            self.panicked = false;
            r
        } else {
            // Write to the buffer. In this case, we write to the buffer even if it fills it
            // exactly. Doing otherwise would mean flushing the buffer, then writing this
            // input to the inner writer, which in many cases would be a worse strategy.

            // SAFETY: There was either enough spare capacity already, or there wasn't and we
            // flushed the buffer to ensure that there is. In the latter case, we know that there
            // is because flushing ensured that our entire buffer is spare capacity, and we entered
            // this block because the input buffer length is less than that capacity. In either
            // case, it's safe to write the input buffer to our buffer.
            unsafe {
                self.write_to_buffer_unchecked(buf);
            }

            Ok(buf.len())
        }
    }

    // Ensure this function does not get inlined into `write_all`, so that it
    // remains inlineable and its common path remains as short as possible.
    // If this function ends up being called frequently relative to `write_all`,
    // it's likely a sign that the client is using an improperly sized buffer
    // or their write patterns are somewhat pathological.
    #[cold]
    #[inline(never)]
    fn write_all_cold(&mut self, buf: &[u8]) -> io::Result<()> {
        // Normally, `write_all` just calls `write` in a loop. We can do better
        // by calling `self.get_mut().write_all()` directly, which avoids
        // round trips through the buffer in the event of a series of partial
        // writes in some circumstances.

        if buf.len() > self.spare_capacity() {
            self.flush_buf()?;
        }

        // Why not len > capacity? To avoid a needless trip through the buffer when the input
        // exactly fills it. We'd just need to flush it to the underlying writer anyway.
        if buf.len() >= self.buf.len() {
            self.panicked = true;
            let r = self.get_mut().write_all(buf);
            self.panicked = false;
            r
        } else {
            // Write to the buffer. In this case, we write to the buffer even if it fills it
            // exactly. Doing otherwise would mean flushing the buffer, then writing this
            // input to the inner writer, which in many cases would be a worse strategy.

            // SAFETY: There was either enough spare capacity already, or there wasn't and we
            // flushed the buffer to ensure that there is. In the latter case, we know that there
            // is because flushing ensured that our entire buffer is spare capacity, and we entered
            // this block because the input buffer length is less than that capacity. In either
            // case, it's safe to write the input buffer to our buffer.
            unsafe {
                self.write_to_buffer_unchecked(buf);
            }

            Ok(())
        }
    }

    // SAFETY: Requires `buf.len() <= self.buf.capacity() - self.buf.len()`,
    // i.e., that input buffer length is less than or equal to spare capacity.
    #[inline]
    unsafe fn write_to_buffer_unchecked(&mut self, buf: &[u8]) {
        debug_assert!(buf.len() <= self.spare_capacity());
        let buf_len = buf.len();
        let src = buf.as_ptr();
        let dst = MaybeUninit::slice_assume_init_mut(&mut self.buf)
            .as_mut_ptr()
            .add(self.end);
        ptr::copy_nonoverlapping(src, dst, buf_len);
        self.end += buf_len;
    }

    #[inline]
    fn spare_capacity(&self) -> usize {
        self.buf.len() - self.end
    }
}

impl<W: Write, const N: usize> Write for StackBufWriter<W, N> {
    #[inline]
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        // Use < instead of <= to avoid a needless trip through the buffer in some cases.
        // See `write_cold` for details.
        if buf.len() < self.spare_capacity() {
            // SAFETY: safe by above conditional.
            unsafe {
                self.write_to_buffer_unchecked(buf);
            }

            Ok(buf.len())
        } else {
            self.write_cold(buf)
        }
    }

    fn write_vectored(&mut self, bufs: &[IoSlice<'_>]) -> io::Result<usize> {
        // FIXME: Consider applying `#[inline]` / `#[inline(never)]` optimizations already applied
        // to `write` and `write_all`. The performance benefits can be significant. See #79930.
        if self.get_ref().is_write_vectored() {
            // We have to handle the possibility that the total length of the buffers overflows
            // `usize` (even though this can only happen if multiple `IoSlice`s reference the
            // same underlying buffer, as otherwise the buffers wouldn't fit in memory). If the
            // computation overflows, then surely the input cannot fit in our buffer, so we forward
            // to the inner writer's `write_vectored` method to let it handle it appropriately.
            let saturated_total_len = bufs
                .iter()
                .fold(0usize, |acc, b| acc.saturating_add(b.len()));

            if saturated_total_len > self.spare_capacity() {
                // Flush if the total length of the input exceeds our buffer's spare capacity.
                // If we would have overflowed, this condition also holds, and we need to flush.
                self.flush_buf()?;
            }

            if saturated_total_len >= self.buf.len() {
                // Forward to our inner writer if the total length of the input is greater than or
                // equal to our buffer capacity. If we would have overflowed, this condition also
                // holds, and we punt to the inner writer.
                self.panicked = true;
                let r = self.get_mut().write_vectored(bufs);
                self.panicked = false;
                r
            } else {
                // `saturated_total_len < self.buf.capacity()` implies that we did not saturate.

                // SAFETY: We checked whether or not the spare capacity was large enough above. If
                // it was, then we're safe already. If it wasn't, we flushed, making sufficient
                // room for any input <= the buffer size, which includes this input.
                unsafe {
                    bufs.iter().for_each(|b| self.write_to_buffer_unchecked(b));
                };

                Ok(saturated_total_len)
            }
        } else {
            let mut iter = bufs.iter();
            let mut total_written = if let Some(buf) = iter.by_ref().find(|&buf| !buf.is_empty()) {
                // This is the first non-empty slice to write, so if it does
                // not fit in the buffer, we still get to flush and proceed.
                if buf.len() > self.spare_capacity() {
                    self.flush_buf()?;
                }
                if buf.len() >= self.buf.len() {
                    // The slice is at least as large as the buffering capacity,
                    // so it's better to write it directly, bypassing the buffer.
                    self.panicked = true;
                    let r = self.get_mut().write(buf);
                    self.panicked = false;
                    return r;
                } else {
                    // SAFETY: We checked whether or not the spare capacity was large enough above.
                    // If it was, then we're safe already. If it wasn't, we flushed, making
                    // sufficient room for any input <= the buffer size, which includes this input.
                    unsafe {
                        self.write_to_buffer_unchecked(buf);
                    }

                    buf.len()
                }
            } else {
                return Ok(0);
            };
            debug_assert!(total_written != 0);
            for buf in iter {
                if buf.len() <= self.spare_capacity() {
                    // SAFETY: safe by above conditional.
                    unsafe {
                        self.write_to_buffer_unchecked(buf);
                    }

                    // This cannot overflow `usize`. If we are here, we've written all of the bytes
                    // so far to our buffer, and we've ensured that we never exceed the buffer's
                    // capacity. Therefore, `total_written` <= `self.buf.capacity()` <= `usize::MAX`.
                    total_written += buf.len();
                } else {
                    break;
                }
            }
            Ok(total_written)
        }
    }

    fn is_write_vectored(&self) -> bool {
        true
    }

    fn flush(&mut self) -> io::Result<()> {
        self.flush_buf().and_then(|()| self.get_mut().flush())
    }

    #[inline]
    fn write_all(&mut self, buf: &[u8]) -> io::Result<()> {
        // Use < instead of <= to avoid a needless trip through the buffer in some cases.
        // See `write_all_cold` for details.
        if buf.len() < self.spare_capacity() {
            // SAFETY: safe by above conditional.
            unsafe {
                self.write_to_buffer_unchecked(buf);
            }

            Ok(())
        } else {
            self.write_all_cold(buf)
        }
    }
}

impl<W: Write, const N: usize> fmt::Debug for StackBufWriter<W, N>
where
    W: fmt::Debug,
{
    fn fmt(&self, fmt: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt.debug_struct("BufWriter")
            .field("writer", &self.inner)
            .field(
                "buffer",
                &format_args!("{}/{}", self.end - self.start, self.spare_capacity()),
            )
            .finish()
    }
}

impl<W: Write + Seek, const N: usize> Seek for StackBufWriter<W, N> {
    /// Seek to the offset, in bytes, in the underlying writer.
    ///
    /// Seeking always writes out the internal buffer before seeking.
    fn seek(&mut self, pos: SeekFrom) -> io::Result<u64> {
        self.flush_buf()?;
        self.get_mut().seek(pos)
    }
}

impl<W: Write, const N: usize> Drop for StackBufWriter<W, N> {
    fn drop(&mut self) {
        if !self.panicked {
            // dtors should not panic, so we ignore a failed flush
            let _r = self.flush_buf();
        }
    }
}
