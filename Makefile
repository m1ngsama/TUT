SHELL = /bin/sh

prefix = /usr/local
exec_prefix = $(prefix)
bindir = $(exec_prefix)/bin

CARGO = cargo
INSTALL = install
INSTALL_PROGRAM = $(INSTALL)
MKDIR_P = $(INSTALL) -d
RM = rm -f
STRIP = strip

PROGRAM = target/release/tut

.PHONY: all check installdirs install install-strip installcheck uninstall mostlyclean clean distclean maintainer-clean

all:
	$(CARGO) build --release --locked

check:
	$(CARGO) test --all-targets --locked

installdirs:
	$(MKDIR_P) "$(DESTDIR)$(bindir)"

install: all installdirs
	$(INSTALL_PROGRAM) -m 755 "$(PROGRAM)" "$(DESTDIR)$(bindir)/tut"

install-strip: install
	$(STRIP) "$(DESTDIR)$(bindir)/tut"

installcheck:
	"$(DESTDIR)$(bindir)/tut" --version >/dev/null

uninstall:
	$(RM) "$(DESTDIR)$(bindir)/tut"

mostlyclean: clean

clean:
	$(CARGO) clean

distclean: clean

maintainer-clean: distclean
