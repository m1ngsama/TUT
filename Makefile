SHELL = /bin/sh

prefix ?= /usr/local
exec_prefix ?= $(prefix)
bindir ?= $(exec_prefix)/bin
datarootdir ?= $(prefix)/share
mandir ?= $(datarootdir)/man
man1dir ?= $(mandir)/man1

CARGO ?= cargo
CARGO_TARGET_DIR ?= target
CARGO_BUILD_TARGET ?=
INSTALL ?= install
INSTALL_PROGRAM ?= $(INSTALL)
INSTALL_DATA ?= $(INSTALL) -m 644
MKDIR_P ?= $(INSTALL) -d
RM ?= rm -f
STRIP ?= strip

CARGO_TARGET_OPTION = $(if $(strip $(CARGO_BUILD_TARGET)),--target $(CARGO_BUILD_TARGET))
TARGET_RELEASE_DIR = $(CARGO_TARGET_DIR)/$(if $(strip $(CARGO_BUILD_TARGET)),$(CARGO_BUILD_TARGET)/)release
PROGRAM = $(TARGET_RELEASE_DIR)/tut
MANPAGE = docs/tut.1
PACKAGE_VERSION = $(shell $(CARGO) pkgid --locked 2>/dev/null | sed 's/.*[@:]//')
DIST_ARCHIVE = $(CARGO_TARGET_DIR)/package/tut-$(PACKAGE_VERSION).crate

.PHONY: all check release-check installdirs install install-strip installcheck uninstall dist distcheck mostlyclean clean distclean maintainer-clean

all:
	CARGO_TARGET_DIR="$(CARGO_TARGET_DIR)" $(CARGO) build --release --locked $(CARGO_TARGET_OPTION)

check:
	CARGO_TARGET_DIR="$(CARGO_TARGET_DIR)" $(CARGO) test --all-targets --locked $(CARGO_TARGET_OPTION)

release-check:
	CARGO_TARGET_DIR="$(CARGO_TARGET_DIR)" $(CARGO) test --release --all-targets --locked $(CARGO_TARGET_OPTION)

installdirs:
	$(MKDIR_P) "$(DESTDIR)$(bindir)"
	$(MKDIR_P) "$(DESTDIR)$(man1dir)"

install: all installdirs
	$(INSTALL_PROGRAM) -m 755 "$(PROGRAM)" "$(DESTDIR)$(bindir)/tut"
	$(INSTALL_DATA) "$(MANPAGE)" "$(DESTDIR)$(man1dir)/tut.1"

install-strip: install
	$(STRIP) "$(DESTDIR)$(bindir)/tut"

installcheck:
	test "$$("$(DESTDIR)$(bindir)/tut" --version | sed -n '1p')" = "tut (TUT) $(PACKAGE_VERSION)"
	"$(DESTDIR)$(bindir)/tut" --help >/dev/null
	test -s "$(DESTDIR)$(man1dir)/tut.1"
	cmp "$(MANPAGE)" "$(DESTDIR)$(man1dir)/tut.1"

uninstall:
	$(RM) "$(DESTDIR)$(bindir)/tut"
	$(RM) "$(DESTDIR)$(man1dir)/tut.1"

dist:
	CARGO_TARGET_DIR="$(CARGO_TARGET_DIR)" $(CARGO) package --locked

distcheck: dist
	@set -eu; \
	work=$$(mktemp -d "$${TMPDIR:-/tmp}/tut-distcheck.XXXXXX"); \
	trap 'rm -rf "$$work"' EXIT HUP INT TERM; \
	mkdir "$$work/source" "$$work/stage"; \
	tar -xzf "$(abspath $(DIST_ARCHIVE))" -C "$$work/source"; \
	cd "$$work/source/tut-$(PACKAGE_VERSION)"; \
	$(MAKE) check release-check CARGO="$(CARGO)" CARGO_TARGET_DIR="$$work/target"; \
	$(MAKE) install installcheck CARGO="$(CARGO)" CARGO_TARGET_DIR="$$work/target" DESTDIR="$$work/stage" prefix=/usr; \
	$(MAKE) uninstall DESTDIR="$$work/stage" prefix=/usr; \
	test ! -e "$$work/stage/usr/bin/tut"; \
	test ! -e "$$work/stage/usr/share/man/man1/tut.1"

mostlyclean: clean

clean:
	CARGO_TARGET_DIR="$(CARGO_TARGET_DIR)" $(CARGO) clean

distclean: clean

maintainer-clean: distclean
