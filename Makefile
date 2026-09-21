.PHONY: install

install:
	cargo build --release
	mkdir -p $(HOME)/.local/bin
	cp target/release/rsloc $(HOME)/.local/bin/.rsloc.tmp
	chmod 0755 $(HOME)/.local/bin/.rsloc.tmp
	mv -f $(HOME)/.local/bin/.rsloc.tmp $(HOME)/.local/bin/rsloc
