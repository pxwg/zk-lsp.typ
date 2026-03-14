class ZkLsp < Formula
  desc "Zettelkasten LSP server and CLI tools for Typst-based wikis"
  homepage "https://github.com/pxwg/zk-lsp.typ"
  license "AGPL-3.0"
  version "0.4.0"

  on_macos do
    on_arm do
      url "https://github.com/pxwg/zk-lsp.typ/releases/download/v0.4.0/zk-lsp-aarch64-apple-darwin.tar.gz"
      sha256 "16fd8d9769bd44dfeb9b1cd4b7f625ac4dfc4c5412917d0810afc075e9e165de"
    end
    on_intel do
      url "https://github.com/pxwg/zk-lsp.typ/releases/download/v0.4.0/zk-lsp-x86_64-apple-darwin.tar.gz"
      sha256 "66e3e37ee1a898d9531518ee729d15544819609e76752cfbaad67f3001372d79"
    end
  end

  on_linux do
    on_arm do
      url "https://github.com/pxwg/zk-lsp.typ/releases/download/v0.4.0/zk-lsp-aarch64-unknown-linux-gnu.tar.gz"
      sha256 "aa3513eb1189a589b30d083ff93110519720baaae3a796d02bde0b4ccdd637c5"
    end
    on_intel do
      url "https://github.com/pxwg/zk-lsp.typ/releases/download/v0.4.0/zk-lsp-x86_64-unknown-linux-gnu.tar.gz"
      sha256 "e241b0bb3801afa7e494be36ba93da6f092010d8fc1a1bcf35ddf8fed87f7577"
    end
  end

  def install
    bin.install "zk-lsp"
  end

  test do
    assert_match "zk-lsp", shell_output("\#<built-in function bin>/zk-lsp --help")
  end
end
