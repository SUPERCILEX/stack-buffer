#![feature(vec_spare_capacity)]
#![feature(maybe_uninit_slice)]

use std::{
    io::{BufReader, BufWriter, Read, Write},
    mem::MaybeUninit,
    slice,
};

use criterion::{
    black_box, criterion_group, criterion_main, AxisScale, BatchSize, BenchmarkId, Criterion,
    PlotConfiguration, Throughput,
};
use rand::{thread_rng, RngCore};

use stack_buffer::{StackBufReader, StackBufWriter};

fn buf_reader(c: &mut Criterion) {
    let mut group = c.benchmark_group("buf_reader");
    group.plot_config(PlotConfiguration::default().summary_scale(AxisScale::Logarithmic));

    for num_bytes in [10, 1_000, 10_000, 100_000, 1_000_000, 10_000_000] {
        let data = gen_random_bytes(num_bytes);

        macro_rules! stack_bench {
            ($alloc:literal) => {
                group.bench_with_input(
                    BenchmarkId::new(stringify!($alloc), num_bytes),
                    &data,
                    |b, data| {
                        b.iter(|| {
                            let reader =
                                StackBufReader::<_, $alloc>::new(black_box(data.as_slice()));
                            for (i, byte) in reader.bytes().enumerate() {
                                assert_eq!(byte.unwrap(), data[i]);
                            }
                        })
                    },
                );
            };
        }

        macro_rules! heap_bench {
            ($alloc:literal) => {
                group.bench_with_input(
                    BenchmarkId::new(format!("{}-heap", $alloc), num_bytes),
                    &data,
                    |b, data| {
                        b.iter(|| {
                            let reader =
                                BufReader::with_capacity($alloc, black_box(data.as_slice()));
                            for (i, byte) in reader.bytes().enumerate() {
                                assert_eq!(byte.unwrap(), data[i]);
                            }
                        })
                    },
                );
            };
        }

        group.throughput(Throughput::Elements(num_bytes));

        stack_bench!(512);
        stack_bench!(4096);
        stack_bench!(8192);
        stack_bench!(65536);

        heap_bench!(512);
        heap_bench!(4096);
        heap_bench!(8192);
        heap_bench!(65536);
    }
}

fn buf_writer(c: &mut Criterion) {
    let mut group = c.benchmark_group("buf_writer");
    group.plot_config(PlotConfiguration::default().summary_scale(AxisScale::Logarithmic));

    for num_bytes in [10, 1_000, 10_000, 100_000, 1_000_000, 10_000_000] {
        let data = gen_random_bytes(num_bytes);

        macro_rules! stack_bench {
            ($alloc:literal) => {
                group.bench_with_input(
                    BenchmarkId::new(stringify!($alloc), num_bytes),
                    &data,
                    |b, data| {
                        b.iter_batched_ref(
                            || create_uninit_vec(num_bytes as usize),
                            |output| {
                                {
                                    let mut writer = StackBufWriter::<_, $alloc>::new(black_box(
                                        output.as_mut_slice(),
                                    ));
                                    for byte in data {
                                        writer.write_all(black_box(slice::from_ref(byte))).unwrap();
                                    }
                                    writer.flush().unwrap();
                                }

                                assert_eq!(data, output);
                            },
                            BatchSize::LargeInput,
                        )
                    },
                );
            };
        }

        macro_rules! heap_bench {
            ($alloc:literal) => {
                group.bench_with_input(
                    BenchmarkId::new(format!("{}-heap", $alloc), num_bytes),
                    &data,
                    |b, data| {
                        b.iter_batched_ref(
                            || create_uninit_vec(num_bytes as usize),
                            |output| {
                                {
                                    let mut writer = BufWriter::with_capacity(
                                        $alloc,
                                        black_box(output.as_mut_slice()),
                                    );
                                    for byte in data {
                                        writer.write_all(black_box(slice::from_ref(byte))).unwrap();
                                    }
                                    writer.flush().unwrap();
                                }

                                assert_eq!(data, output);
                            },
                            BatchSize::LargeInput,
                        )
                    },
                );
            };
        }

        group.throughput(Throughput::Elements(num_bytes));

        stack_bench!(512);
        stack_bench!(4096);
        stack_bench!(8192);
        stack_bench!(65536);

        heap_bench!(512);
        heap_bench!(4096);
        heap_bench!(8192);
        heap_bench!(65536);
    }
}

fn gen_random_bytes(num_bytes: u64) -> Vec<u8> {
    let mut data = Vec::with_capacity(num_bytes as usize);
    let raw_data = data.spare_capacity_mut();
    thread_rng().fill_bytes(unsafe { MaybeUninit::slice_assume_init_mut(raw_data) });
    unsafe {
        data.set_len(data.capacity());
    }
    data
}

#[allow(clippy::uninit_vec)]
fn create_uninit_vec(num_bytes: usize) -> Vec<u8> {
    let mut out = Vec::with_capacity(num_bytes);
    unsafe {
        out.set_len(num_bytes);
    }
    out
}

criterion_group! {
    name = benches;
    config = Criterion::default();
    targets =
    buf_reader,
    buf_writer,
}
criterion_main!(benches);
