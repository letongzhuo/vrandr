# Maintainer: tongzhuo <https://github.com/tongzhuo>
pkgname=vrandr
pkgver=0.1.0
pkgrel=1
pkgdesc='Vim-style TUI for xrandr, built with Ratatui'
arch=('x86_64' 'aarch64')
url='https://github.com/tongzhuo/vrandr'
license=('MIT')
depends=('xrandr')
makedepends=('cargo' 'rust')
source=("$pkgname-$pkgver.tar.gz::https://github.com/tongzhuo/vrandr/archive/refs/tags/v$pkgver.tar.gz")
sha256sums=('SKIP')

prepare() {
	cd "$pkgname-$pkgver"
	cargo fetch --locked --target "$(rustc -vV | sed -n 's/host: //p')"
}

build() {
	cd "$pkgname-$pkgver"
	export CARGO_TARGET_DIR=target
	cargo build --release --frozen
}

check() {
	cd "$pkgname-$pkgver"
	cargo test --frozen
}

package() {
	cd "$pkgname-$pkgver"
	install -Dm0755 target/release/vrandr "$pkgdir/usr/bin/vrandr"
	install -Dm0644 LICENSE "$pkgdir/usr/share/licenses/$pkgname/LICENSE"
	install -Dm0644 README.md "$pkgdir/usr/share/doc/$pkgname/README.md"
	install -Dm0644 README.zh-CN.md "$pkgdir/usr/share/doc/$pkgname/README.zh-CN.md"
}
