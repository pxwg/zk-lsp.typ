class ZkLsp < Formula
  desc "Zettelkasten LSP server and CLI tools for Typst-based wikis"
  homepage "https://github.com/pxwg/zk-lsp.typ"
  license "AGPL-3.0"
  version "0.4.1"

  on_macos do
    on_arm do
      url "https://github.com/pxwg/zk-lsp.typ/releases/download/v0.4.1/zk-lsp-aarch64-apple-darwin.tar.gz"
      sha256 "4dc159f531dd698b7a8fb83ad6a6a7ba5edbeacbd2eb678458e3636a219880c2"
    end
    on_intel do
      url "https://github.com/pxwg/zk-lsp.typ/releases/download/v0.4.1/zk-lsp-x86_64-apple-darwin.tar.gz"
      sha256 "6c610a8a40d23da412ca87514f86cf1c045167b21c0f8306cce719e3df9fb59d"
    end
  end

  on_linux do
    on_arm do
      url "https://github.com/pxwg/zk-lsp.typ/releases/download/v0.4.1/zk-lsp-aarch64-unknown-linux-gnu.tar.gz"
      sha256 "cf183da34ddcebbbeb4c8b1aeefffd47bf47e69b560ca6abee56d12f002cb8c1"
    end
    on_intel do
      url "https://github.com/pxwg/zk-lsp.typ/releases/download/v0.4.1/zk-lsp-x86_64-unknown-linux-gnu.tar.gz"
      sha256 "1389cc651611d8f0e14bbf00b7cabbc4d0a641300250bcf9d2b60fefc011dbec"
    end
  end

  def install
    bin.install "zk-lsp"
  end

  test do
    assert_match "zk-lsp", shell_output("\#<built-in function bin>/zk-lsp --help")
  end
end
