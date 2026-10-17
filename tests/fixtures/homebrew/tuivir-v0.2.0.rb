class Tuivir < Formula
  desc "Inspect and operate local virtualization and container providers"
  homepage "https://github.com/saif97/Tuivir"
  version "0.2.0"
  license "MIT OR Apache-2.0"

  on_macos do
    on_arm do
      url "https://github.com/saif97/Tuivir/releases/download/v0.2.0/tuivir-v0.2.0-aarch64-apple-darwin.tar.gz"
      sha256 "43590cef770d948e630d35409dabb2a89b94ffb461a49d70be8a401672e451df"
    end

    on_intel do
      url "https://github.com/saif97/Tuivir/releases/download/v0.2.0/tuivir-v0.2.0-x86_64-apple-darwin.tar.gz"
      sha256 "703910b55af80f82935c51df1d721a36dc05c8223e4a89eb73363dff1edc14d4"
    end
  end

  def install
    bin.install "tuivir"
  end

  test do
    assert_equal "0.2.0\n", shell_output("#{bin}/tuivir --version")
  end
end
