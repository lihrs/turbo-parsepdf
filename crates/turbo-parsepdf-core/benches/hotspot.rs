//! Phase-by-phase hotspot profiler.
//!
//! Times each parser stage in isolation so optimization is driven by measured
//! cost, not guesswork. Run:
//! `cargo bench --features bench-internals --bench hotspot`.
//!
//! Stages grow as the pipeline lands (xref → content interpret → font decode →
//! layout). Phase 1 covers the two lowest-level hot loops: the COS lexer and the
//! `FlateDecode` inflater.

use criterion::{criterion_group, criterion_main, Criterion};
use std::hint::black_box;

use turbo_parsepdf_core::cos::parse_object;
use turbo_parsepdf_core::filter::flate_decode;
use turbo_parsepdf_core::lex::Lexer;

/// A representative content-ish COS value (nested arrays + dict + strings).
const COS_SAMPLE: &[u8] =
    b"<< /Type /Page /MediaBox [0 0 612 792] /Contents 4 0 R /Resources << /Font << /F1 5 0 R >> >> >>";

/// A zlib stream of "Hi" (header + stored block + dummy adler).
const ZLIB_SAMPLE: [u8; 13] = [
    0x78, 0x9c, 0x01, 0x02, 0x00, 0xfd, 0xff, b'H', b'i', 0x00, 0x62, 0x00, 0x62,
];

fn bench_lex(c: &mut Criterion) {
    c.bench_function("cos/parse_object", |b| {
        b.iter(|| {
            let mut lx = Lexer::new(black_box(COS_SAMPLE));
            black_box(parse_object(&mut lx).unwrap())
        })
    });
}

fn bench_inflate(c: &mut Criterion) {
    c.bench_function("filter/flate_decode", |b| {
        b.iter(|| black_box(flate_decode(black_box(&ZLIB_SAMPLE)).unwrap()))
    });
}

criterion_group!(hotspot, bench_lex, bench_inflate);
criterion_main!(hotspot);
