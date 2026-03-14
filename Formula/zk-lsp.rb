class ZkLsp < Formula
  desc "Zettelkasten LSP server and CLI tools for Typst-based wikis"
  homepage "https://github.com/pxwg/zk-lsp.typ"
  license "AGPL-3.0"
  version "0.4.0"

  on_macos do
    on_arm do
      url "https://github.com/pxwg/zk-lsp.typ/releases/download/v0.4.0/zk-lsp-aarch64-apple-darwin.tar.gz"
      sha256 "b1a814f56a6ccf261bb068643430f58914a8bc47042f45e5372ca6985cdb250f"
    end
    on_intel do
      url "https://github.com/pxwg/zk-lsp.typ/releases/download/v0.4.0/zk-lsp-x86_64-apple-darwin.tar.gz"
      sha256 "08458046724f1c3ffa6224824be32183845c984cd4cb0e19d93184075ca2e17a"
    end
  end

  on_linux do
    on_arm do
      url "https://github.com/pxwg/zk-lsp.typ/releases/download/v0.4.0/zk-lsp-aarch64-unknown-linux-gnu.tar.gz"
      sha256 "010482dc9cd6cff1617790067323069e08f57ffb89554ce96d9fe00b1ca662ae"
    end
    on_intel do
      url "https://github.com/pxwg/zk-lsp.typ/releases/download/v0.4.0/zk-lsp-x86_64-unknown-linux-gnu.tar.gz"
      sha256 "d5f93f5bf170e5a81963c8dd50634b29b818fa38dd52427872eac46d426545e4"
    end
  end

  def install
    bin.install "zk-lsp"
  end

  test do
    assert_match "zk-lsp", shell_output("\#<built-in function bin>/zk-lsp --help")
  end
end
