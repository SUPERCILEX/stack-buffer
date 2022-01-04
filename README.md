# StackBuf{Reader,Writer}

Stack buffer provides alternatives to BufReader and BufWriter allocated on the stack instead of the
heap. Its implementation is mostly copied from the standard library and changed only when required
to use the stack.

## Usage

> The nightly compiler is required.

Add the following to your `Cargo.toml` file:

```toml
[dependencies]
stack-buffer = "0.1.0"
```

Replace `Buf{Reader,Writer}::{new,with_capacity}` with `StackBuf{Reader,Writer}::<_, N>::new`
where `N` is the allocated size on the stack.

## Performance

`StackBufReader` always outperforms `BufReader`, typically by about 6%. Furthermore, the best stack
size appears to be `4096` for reads under 1 MB and `8192` for 1 MB+ reads.

`StackBufWriter` does not appear to offer a significant benefit over `BufWriter` and is sometimes
worse, but this may be due to benchmarking inaccuracies. `8192` appears to consistently be the best
stack size.
