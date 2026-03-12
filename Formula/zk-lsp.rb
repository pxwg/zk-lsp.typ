class ZkLsp < Formula
  desc "Zettelkasten LSP server and CLI tools for Typst-based wikis"
  homepage "https://github.com/pxwg/zk-lsp.typst"
  license "AGPL-3.0"

  stable do
    url "https://github.com/pxwg/zk-lsp.typst/archive/refs/tags/v0.3.0.tar.gz"
    sha256 "PLACEHOLDER_RUN_brew_fetch_TO_OBTAIN"
  end

  head "https://github.com/pxwg/zk-lsp.typst.git", branch: "main"

  depends_on "rust" => :build

  def install
    system "cargo", "install", *std_cargo_args
  end

  test do
    assert_match "zk-lsp", shell_output("#{bin}/zk-lsp --help")
  end
end
