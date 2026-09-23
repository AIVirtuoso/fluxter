pkgname=fluxter-git
pkgver=r0.0000000
pkgrel=1
pkgdesc="A terminal-based chat client for the Fluxer messaging platform (git version)"
arch=('x86_64')
url="https://github.com/AIVirtuoso/fluxter"
license=('GPL-3.0-or-later')
depends=('gcc-libs')
makedepends=('cargo' 'git' 'go')
optdepends=('ffmpeg: microphone and playback for voice calls (fluxter-phone)'
            'mpv: playback for voice calls when ffplay is missing'
            'gst-plugins-ugly: screen sharing in voice calls (x264enc)'
            'gst-plugin-pipewire: screen sharing in voice calls (pipewiresrc)'
            'xdg-desktop-portal: screen sharing in voice calls (the screen chooser)')
provides=('fluxter')
conflicts=('fluxter')
options=('!lto')
source=("fluxter::git+https://github.com/AIVirtuoso/fluxter.git")
sha256sums=('SKIP')

pkgver() {
    cd "$srcdir/fluxter"
    printf "r%s.%s" \
        "$(git rev-list --count HEAD)" \
        "$(git rev-parse --short=7 HEAD)"
}

prepare() {
    cd "$srcdir/fluxter/phone"
    export GOPATH="$srcdir/go"
    go mod download -modcacherw
}

build() {
    cd "$srcdir/fluxter"
    export RUSTFLAGS="${RUSTFLAGS} -C link-arg=-fuse-ld=bfd"
    cargo build --release --locked

    # fluxter-phone carries a voice call's sound; without it the client
    # joins a call but nothing is heard or sent
    cd phone
    export GOPATH="$srcdir/go"
    export CGO_ENABLED=0
    go build -trimpath -buildmode=pie -mod=readonly -modcacherw -o fluxter-phone .
}

package() {
    cd "$srcdir/fluxter"

    install -Dm755 \
        "target/release/fluxter" \
        "$pkgdir/usr/bin/fluxter"

    install -Dm755 \
        "phone/fluxter-phone" \
        "$pkgdir/usr/bin/fluxter-phone"

    install -Dm644 \
        "README.md" \
        "$pkgdir/usr/share/doc/$pkgname/README.md"
}
