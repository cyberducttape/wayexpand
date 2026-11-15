# Fedora Copr "make_srpm" entry point. Copr clones this repository and runs
#
#   make -f .copr/Makefile srpm outdir=<dir> spec=<file>
#
# as root in a network-enabled chroot. The spec builds offline from the
# vendored source archive (Source0), so this target produces that archive
# from the checkout, as the release workflow does, and wraps it in an SRPM.
# The RPM build itself then needs no network access. scripts/copr-srpm.sh
# names the build: X.Y.Z-1 at the release tag, X.Y.Z-1.N.gitSHA after it.
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
	sh scripts/copr-srpm.sh "$(outdir)" "$(spec)"
