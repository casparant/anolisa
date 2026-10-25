# AW Preview packaging

[中文版](README_zh.md)

Distribution-owned builder for the historical AW Preview archives. It combines
AW, selected Provider bridges and backend programs for standalone integration
acceptance. Complete ANOLISA releases use the separate
[product packaging entrypoint](../../README.md).

From the repository root on Linux:

```bash
cargo run --locked --release --manifest-path distribution/tooling/aw-preview/Cargo.toml \
  --bin anolisa-aw-preview -- --version 0.1.0-preview.1 --component all \
  --output "$PWD/target/aw-preview-packages"
```

`--component core` is the default; `sec-core` and `tokenless` produce the
selected extension; `all` additionally produces all-in-one. The historical
`aw-build` Cargo binary forwards to this builder from a source checkout.

`templates/` owns concrete sec-core, Tokenless and combined configuration
defaults. Provider bundles carry their corresponding `templates/<name>.json`;
all-in-one carries all three. AW reads one explicitly selected file through
`aw config init --template ABS_FILE`, with repeated
`--template-var NAME=VALUE` for literal values. Backend-specific names are not
built into the AW configuration generator. See the
[AW package reference](../../../docs/user-guide/en/aw/packages.md).
