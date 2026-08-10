PREFIX  ?= /usr/local
BINDIR  := $(PREFIX)/bin
BIN     := target/release/optix
MSG     := target/release/optix-msg
SRCS    := $(shell find src -type f -name '*.rs') Cargo.toml Cargo.lock

.PHONY: build deploy install uninstall clean

# Build release binaries, then install them into $(BINDIR).
build: install

# Build release binaries, then install them system-wide via sudo.
# Use this to deploy to a root-owned path like /usr/local/bin in one step.
deploy: $(BIN) $(MSG)
	sudo $(MAKE) install

# Compile the release binaries; rebuilds automatically when sources change.
$(BIN) $(MSG): $(SRCS)
	cargo build --release

install: $(BIN) $(MSG)
	install -Dm755 $(BIN) $(DESTDIR)$(BINDIR)/optix
	install -Dm755 $(MSG) $(DESTDIR)$(BINDIR)/optix-msg

uninstall:
	rm -f $(DESTDIR)$(BINDIR)/optix
	rm -f $(DESTDIR)$(BINDIR)/optix-msg

clean:
	cargo clean
