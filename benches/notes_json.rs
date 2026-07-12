use std::fmt::Write as _;
use std::fs;

use criterion::{criterion_group, criterion_main, BenchmarkId, Criterion};
use tempfile::TempDir;
use zk_lsp::config::WikiConfig;
use zk_lsp::note_info::build_notes_json;

fn fixture(note_count: usize) -> (TempDir, WikiConfig) {
    let root = tempfile::tempdir().expect("create benchmark wiki");
    fs::create_dir(root.path().join("note")).expect("create note directory");

    let mut metadata = String::from("format-version = 1\n\n");
    for index in 0..note_count {
        let id = format!("{:010}", index);
        let note = format!(
            "#let zk-metadata = zk_metadata(\"{id}\")\n\
             #show: zettel.with(metadata: zk-metadata)\n\n\
             = Benchmark note {index} <{id}>\n\n\
             Synthetic benchmark body.\n"
        );
        fs::write(root.path().join("note").join(format!("{id}.typ")), note)
            .expect("write benchmark note");
        writeln!(
            metadata,
            "[notes.\"{id}\"]\n\
             schema-version = 1\n\
             aliases = []\n\
             abstract = \"\"\n\
             keywords = []\n\
             generated = false\n\
             checklist-status = \"none\"\n\
             relation = \"active\"\n\
             relation-target = []\n"
        )
        .expect("append metadata");
    }
    fs::write(root.path().join("metadata.toml"), metadata).expect("write metadata");
    let config = WikiConfig::from_root(root.path().to_path_buf());
    (root, config)
}

fn notes_json(c: &mut Criterion) {
    let runtime = tokio::runtime::Runtime::new().expect("create Tokio runtime");
    let mut group = c.benchmark_group("notes_json");
    for note_count in [100, 1_000] {
        let (_root, config) = fixture(note_count);
        group.bench_with_input(
            BenchmarkId::from_parameter(note_count),
            &note_count,
            |b, _| {
                b.to_async(&runtime)
                    .iter(|| async { build_notes_json(&config).await.unwrap() });
            },
        );
    }
    group.finish();
}

criterion_group!(benches, notes_json);
criterion_main!(benches);
