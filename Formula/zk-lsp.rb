class ZkLsp < Formula
  desc "Zettelkasten LSP server and CLI tools for Typst-based wikis"
  homepage "https://github.com/pxwg/zk-lsp.typ"
  license "AGPL-3.0"
  version "0.5.4"

  on_macos do
    on_arm do
      url "https://github.com/pxwg/zk-lsp.typ/releases/download/v0.5.4/zk-lsp-aarch64-apple-darwin.tar.gz"
      sha256 "17507bf8ff6a5e9d17f7c5b6dcc459199d130aa42b04855f4563acdc2c67e9cb"
    end
    on_intel do
      url "https://github.com/pxwg/zk-lsp.typ/releases/download/v0.5.4/zk-lsp-x86_64-apple-darwin.tar.gz"
      sha256 "27c4e57d78803c0cd56a1ae92d8f36eec9565db24246c5126dd523861f749856"
    end
  end

  on_linux do
    on_arm do
      url "https://github.com/pxwg/zk-lsp.typ/releases/download/v0.5.4/zk-lsp-aarch64-unknown-linux-gnu.tar.gz"
      sha256 "1eae319c4c5ec91c64293b2cd973b39238457fb28116eb668327103d9e766de6"
    end
    on_intel do
      url "https://github.com/pxwg/zk-lsp.typ/releases/download/v0.5.4/zk-lsp-x86_64-unknown-linux-gnu.tar.gz"
      sha256 "04d7a5da6786b695a257b31ce4f5d9458c44eff7e2a44a9e861761a2ff512914"
    end
  end

  def install
    bin.install "zk-lsp"
  end

  test do
    assert_match "zk-lsp", shell_output("#{bin}/zk-lsp --help")
  end
end
