#![expect(clippy::unwrap_used)]
use criterion::{Criterion, criterion_group, criterion_main};
use mdfrier::{DefaultMapper, MdFrier};
use std::hint::black_box;

fn bench_full_md(c: &mut Criterion) {
    let input = std::fs::read_to_string("assets/full.md").unwrap();

    c.bench_function("parse full.md", |b| {
        b.iter(|| {
            let mut frier = MdFrier::new().unwrap();
            frier
                .parse(black_box(80), black_box(input.as_str()), &DefaultMapper)
                .unwrap()
                .collect::<Vec<_>>()
        })
    });
}

criterion_group!(benches, bench_full_md);
criterion_main!(benches);
