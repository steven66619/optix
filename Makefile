PREFIX  ?= /usr/local
BINDIR  := $(PREFIX)/bin
BIN     := target/release/optix
SRCS    := $(shell find src -type f -name '*.rs') Cargo.toml Cargo.lock

.PHONY: build deploy install uninstall clean

# Build a release binary, then install it into $(BINDIR).
build: install

# Build a release binary, then install it system-wide via sudo.
# Use this to deploy to a root-owned path like /usr/local/bin in one step.
deploy: $(BIN)
	sudo $(MAKE) install

# Compile the release binary; rebuilds automatically when sources change.
$(BIN): $(SRCS)
	cargo build --release

install: $(BIN)
	install -Dm755 $(BIN) $(DESTDIR)$(BINDIR)/optix

uninstall:
	rm -f $(DESTDIR)$(BINDIR)/optix

clean:
	cargo clean
