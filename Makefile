PREFIX ?= /usr/local
BINDIR := $(PREFIX)/bin
BIN    := target/release/oterminal

.PHONY: install uninstall

install: $(BIN)
	install -Dm755 $(BIN) $(DESTDIR)$(BINDIR)/oterminal

uninstall:
	rm -f $(DESTDIR)$(BINDIR)/oterminal

$(BIN):
	@echo "Release binary missing; run 'cargo build --release' first"
	@exit 1
