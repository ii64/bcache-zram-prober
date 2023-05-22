build:
	cargo build --release

install: build
	sudo install bcache-zram-prober.service /etc/systemd/system
	sudo install target/release/bcache-zram-prober /usr/bin
	sudo chmod 644 /etc/systemd/system/bcache-zram-prober.service

test-svc:
	sudo systemctl daemon-reload
	sudo systemctl start bcache-zram-prober.service
	sudo systemctl status bcache-zram-prober.service