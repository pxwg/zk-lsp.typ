class ZkLsp < Formula
  desc "Zettelkasten LSP server and CLI tools for Typst-based wikis"
  homepage "https://github.com/pxwg/zk-lsp.typ"
  license "AGPL-3.0"
  version "0.5.0"

  on_macos do
    on_arm do
      url "https://github.com/pxwg/zk-lsp.typ/releases/download/v0.5.0/zk-lsp-aarch64-apple-darwin.tar.gz"
      sha256 "bba6e5652e7e5c11b646150f8c3c567610eef86cb812efcab0f0205e117f4cd1"
    end
    on_intel do
      url "https://github.com/pxwg/zk-lsp.typ/releases/download/v0.5.0/zk-lsp-x86_64-apple-darwin.tar.gz"
      sha256 "0a3130e7bbe483d08d6c6fbc0ac95972291a38b6eeaf0ac359e396c6f9851c77"
    end
  end

  on_linux do
    on_arm do
      url "https://github.com/pxwg/zk-lsp.typ/releases/download/v0.5.0/zk-lsp-aarch64-unknown-linux-gnu.tar.gz"
      sha256 "e153f26a183faca85a26d953ba38c2fadb9c834e198ab95625c57f7611cc4084"
    end
    on_intel do
      url "https://github.com/pxwg/zk-lsp.typ/releases/download/v0.5.0/zk-lsp-x86_64-unknown-linux-gnu.tar.gz"
      sha256 "ac2291ad63d290a05eb90203ac726ced1847ec1ac25ab0acd6b034ed7eb48763"
    end
  end

  def install
    bin.install "zk-lsp"
  end

  test do
    assert_match "zk-lsp", shell_output("\#<built-in function bin>/zk-lsp --help")
  end
end
