# Fedora Copr "make_srpm" entry point. Copr clones this repository and runs
#
#   make -f .copr/Makefile srpm outdir=<dir> spec=<file>
#
# as root in a network-enabled chroot. The spec builds offline from the
# vendored source archive (Source0), so this target produces that archive
# from the checkout, as the release workflow does, and wraps it in an SRPM.
# The RPM build itself then needs no network access.
#
# Locally: make -f .copr/Makefile srpm outdir=/tmp/wayexpand-srpm

outdir ?= $(CURDIR)/copr-out
spec ?= wayexpand.spec

.PHONY: srpm
srpm:
	command -v cargo >/dev/null && command -v git >/dev/null && command -v rpmbuild >/dev/null \
		|| dnf -y install cargo git rpm-build tar gzip
	# Copr's checkout is owned by another user than the one running make.
	git config --global --add safe.directory "$(CURDIR)"
	set -eu; \
	top=$$(git rev-parse --show-toplevel); \
	version=$$(sed -n 's/^Version:[[:space:]]*//p' "$$top/$(spec)"); \
	test -n "$$version"; \
	work=$$(mktemp -d); \
	trap 'rm -rf "$$work"' EXIT; \
	cd "$$top"; \
	bash scripts/generate-release-tarballs.sh "$$version"; \
	mkdir -p "$(outdir)"; \
	mv "wayexpand-$$version-vendored.tar.gz" "$$work/"; \
	rm -f "wayexpand-$$version.tar.gz" "wayexpand-$$version"*.sha256; \
	rpmbuild -bs \
		--define "_sourcedir $$work" \
		--define "_srcrpmdir $(outdir)" \
		"$$top/$(spec)"
