class ZkLsp < Formula
  desc "Zettelkasten LSP server and CLI tools for Typst-based wikis"
  homepage "https://github.com/pxwg/zk-lsp.typ"
  license "AGPL-3.0"
  version "0.5.3"

  on_macos do
    on_arm do
      url "https://github.com/pxwg/zk-lsp.typ/releases/download/v0.5.3/zk-lsp-aarch64-apple-darwin.tar.gz"
      sha256 "258888f6bf40b4dd9df446b30d2de096c6fc0b56bb33c38439ef4f8e6a64d938"
    end
    on_intel do
      url "https://github.com/pxwg/zk-lsp.typ/releases/download/v0.5.3/zk-lsp-x86_64-apple-darwin.tar.gz"
      sha256 "b53d27d2a4a29d6dc7640916c5dc62fa5c3a7dafede504b5d7846310c4dcb533"
    end
  end

  on_linux do
    on_arm do
      url "https://github.com/pxwg/zk-lsp.typ/releases/download/v0.5.3/zk-lsp-aarch64-unknown-linux-gnu.tar.gz"
      sha256 "675d5215506bab075bfa02cdc444cd998276b646dab5d1dcb32ed1b0763059db"
    end
    on_intel do
      url "https://github.com/pxwg/zk-lsp.typ/releases/download/v0.5.3/zk-lsp-x86_64-unknown-linux-gnu.tar.gz"
      sha256 "0975449aa4e36e28c05bbafa49811af3c075197e5edef3b46b9154666c015770"
    end
  end

  def install
    bin.install "zk-lsp"
  end

  test do
    assert_match "zk-lsp", shell_output("#{bin}/zk-lsp --help")
  end
end
