//! Extraction cost, no browser: the zero-LLM path is the one every batch row
//! pays for, so it gets a bench that runs anywhere (`cargo bench -p webrain-core`).
//!
//! Sizes are 1K and 10K records. A regression here is invisible in wall-clock
//! batch numbers, which a single slow page can hide.

use criterion::{BenchmarkId, Criterion, Throughput, black_box, criterion_group, criterion_main};
use serde_json::json;
use webrain_core::engines::{autoschema_candidates, bm25_filter, extract_fields};

/// A listing page shaped like the ones autoschema is meant to discover.
fn listing(cards: usize) -> String {
    let mut html = String::with_capacity(cards * 260 + 256);
    html.push_str("<!doctype html><html><head><title>listing</title></head><body><main>");
    for i in 0..cards {
        let price = (i % 90) + 10;
        html.push_str(&format!(
            r#"<div class="product" data-sku="S{i}"><a href="/p/{i}" class="name">Item {i}</a><span class="price">{price}.99</span><img src="/i/{i}.jpg" alt="Item {i}"><p class="desc">Short description of item {i} with a few words.</p></div>"#,
        ));
    }
    html.push_str("</main></body></html>");
    html
}

fn schema() -> Vec<serde_json::Value> {
    vec![
        json!({"name": "name", "selector": "a.name", "type": "text"}),
        json!({"name": "price", "selector": ".price", "type": "text"}),
        json!({"name": "link", "selector": "a.name", "type": "attr", "attr": "href"}),
        json!({"name": "sku", "selector": "", "type": "attr", "attr": "data-sku"}),
    ]
}

fn paragraphs(n: usize) -> Vec<String> {
    (0..n)
        .map(|i| {
            format!(
                "Record {i}: rust browser automation over CDP extracts structured data without calling a model, which keeps the cost predictable and the latency low."
            )
        })
        .collect()
}

fn benches(c: &mut Criterion) {
    let mut g = c.benchmark_group("extract");
    for cards in [1_000usize, 10_000] {
        let html = listing(cards);
        g.throughput(Throughput::Elements(cards as u64));
        g.bench_with_input(BenchmarkId::new("autoschema", cards), &html, |b, html| {
            b.iter(|| autoschema_candidates(black_box(html), black_box(3)))
        });
        g.bench_with_input(
            BenchmarkId::new("schema_fields", cards),
            &html,
            |b, html| {
                let fields = schema();
                b.iter(|| {
                    extract_fields(
                        black_box(html),
                        black_box("div.product"),
                        black_box(&fields),
                    )
                })
            },
        );
    }
    g.finish();

    let mut g = c.benchmark_group("bm25");
    for n in [1_000usize, 10_000] {
        let items = paragraphs(n);
        g.throughput(Throughput::Elements(n as u64));
        g.bench_with_input(BenchmarkId::from_parameter(n), &items, |b, items| {
            b.iter(|| {
                bm25_filter(
                    black_box(items),
                    black_box("rust automation cost"),
                    black_box(20),
                )
            })
        });
    }
    g.finish();
}

criterion_group!(benches_group, benches);
criterion_main!(benches_group);
