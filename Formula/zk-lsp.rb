class ZkLsp < Formula
  desc "Zettelkasten LSP server and CLI tools for Typst-based wikis"
  homepage "https://github.com/pxwg/zk-lsp.typ"
  license "AGPL-3.0"
  version "0.5.2"

  on_macos do
    on_arm do
      url "https://github.com/pxwg/zk-lsp.typ/releases/download/v0.5.2/zk-lsp-aarch64-apple-darwin.tar.gz"
      sha256 "b07fc4dcd995019faa39debd4a10e47ec4174a80ea9253a2acfdc12ae46c6dec"
    end
    on_intel do
      url "https://github.com/pxwg/zk-lsp.typ/releases/download/v0.5.2/zk-lsp-x86_64-apple-darwin.tar.gz"
      sha256 "8d3357d6ceab10fa2c50231c9438c8583d848f873da59975052fdec9fbe4bcca"
    end
  end

  on_linux do
    on_arm do
      url "https://github.com/pxwg/zk-lsp.typ/releases/download/v0.5.2/zk-lsp-aarch64-unknown-linux-gnu.tar.gz"
      sha256 "74b436f6a2458c6f57ac919cbfd8f29764df051dd48d347f03d721175bcd91e8"
    end
    on_intel do
      url "https://github.com/pxwg/zk-lsp.typ/releases/download/v0.5.2/zk-lsp-x86_64-unknown-linux-gnu.tar.gz"
      sha256 "8adcdca9b3edfc414bb264dde15c3cf72defce003faccc4095924576821b0875"
    end
  end

  def install
    bin.install "zk-lsp"
  end

  test do
    assert_match "zk-lsp", shell_output("#{bin}/zk-lsp --help")
  end
end
