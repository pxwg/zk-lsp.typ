class ZkLsp < Formula
  desc "Zettelkasten LSP server and CLI tools for Typst-based wikis"
  homepage "https://github.com/pxwg/zk-lsp.typ"
  license "AGPL-3.0"
  version "0.5.1"

  on_macos do
    on_arm do
      url "https://github.com/pxwg/zk-lsp.typ/releases/download/v0.5.1/zk-lsp-aarch64-apple-darwin.tar.gz"
      sha256 "a463f251ac9e6112513d7e5572d7d57495713882d5a732e6213ee52328595e78"
    end
    on_intel do
      url "https://github.com/pxwg/zk-lsp.typ/releases/download/v0.5.1/zk-lsp-x86_64-apple-darwin.tar.gz"
      sha256 "9989f559650355720a7c075238c7212aa2e3694e67f7c72b86bd43ea747a90c4"
    end
  end

  on_linux do
    on_arm do
      url "https://github.com/pxwg/zk-lsp.typ/releases/download/v0.5.1/zk-lsp-aarch64-unknown-linux-gnu.tar.gz"
      sha256 "e979c5043732454022aa61c58483e42805a6785fd2493acc4999c70a3dd72b31"
    end
    on_intel do
      url "https://github.com/pxwg/zk-lsp.typ/releases/download/v0.5.1/zk-lsp-x86_64-unknown-linux-gnu.tar.gz"
      sha256 "d09c3e6c7c971a2da4edaac92be8e6537dda01e6eb900df0ab7060da7d6b83c3"
    end
  end

  def install
    bin.install "zk-lsp"
  end

  test do
    assert_match "zk-lsp", shell_output("\#<built-in function bin>/zk-lsp --help")
  end
end
