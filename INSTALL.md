# Installing zk-lsp

## macOS — Homebrew

This repository doubles as a Homebrew tap. Add it once, then install normally:

```bash
brew tap pxwg/zk-lsp https://github.com/pxwg/zk-lsp.typst
brew install zk-lsp
```

`rust` is listed as a build dependency and will be pulled in automatically if not
already present. The build compiles from source; expect it to take a minute or two
on first install.

To upgrade after a new release:

```bash
brew upgrade zk-lsp
```

To install the latest unreleased commit from `main`:

```bash
brew install --HEAD pxwg/zk-lsp/zk-lsp
```

---

## Linux / other Unix — build from source

Requires **Rust 1.75+** (`rustup` recommended).

```bash
git clone https://github.com/pxwg/zk-lsp.typst
cd zk-lsp.typst
cargo build --release
```

Then symlink the binary somewhere on your `$PATH`:

```bash
ln -sf "$(pwd)/target/release/zk-lsp" ~/.local/bin/zk-lsp
```

Or install directly with Cargo (no clone needed):

```bash
cargo install --git https://github.com/pxwg/zk-lsp.typst
```

---

## Verifying the install

```bash
zk-lsp --help
```

---

## Maintainer: updating the formula for a new release

1. Tag the release:

   ```bash
   git tag v0.x.0
   git push origin v0.x.0
   ```

2. Obtain the sha256 of the release tarball:

   ```bash
   brew fetch --build-from-source pxwg/zk-lsp/zk-lsp
   # or manually:
   curl -sL https://github.com/pxwg/zk-lsp.typst/archive/refs/tags/v0.x.0.tar.gz \
     | shasum -a 256
   ```

3. Update `Formula/zk-lsp.rb`:
   - Bump the version in the `url` line.
   - Replace `sha256` with the value from step 2.

4. Commit and push — Homebrew users pick up the update on `brew upgrade`.
