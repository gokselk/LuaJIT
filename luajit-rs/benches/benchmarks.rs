use criterion::{criterion_group, criterion_main, Criterion};

fn benchmark_parse(c: &mut Criterion) {
    c.bench_function("parse simple", |b| {
        b.iter(|| {
            luajit_rs::parse("local x = 1 + 2", "bench").unwrap()
        })
    });
}

criterion_group\!(benches, benchmark_parse);
criterion_main\!(benches);
