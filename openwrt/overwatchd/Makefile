# SPDX-License-Identifier: GPL-2.0-only

include $(TOPDIR)/rules.mk

PKG_NAME:=overwatchd
PKG_VERSION:=0.1.0
PKG_RELEASE:=1

# Local-checkout source: this repo has no public git remote/release tag yet.
# Once it does, replace this whole block with:
#   PKG_SOURCE:=$(PKG_NAME)-$(PKG_VERSION).tar.gz
#   PKG_SOURCE_URL:=https://github.com/<org>/luci-app-overwatch/releases/download/v$(PKG_VERSION)
#   PKG_HASH:=<sha256>
# `PKG_SOURCE_PROTO:=file` + `PKG_SOURCE_LOCAL` (the documented mechanism for
# a local source directory) was tried first and does NOT populate
# PKG_BUILD_DIR as its name implies — verified against a real build: the
# resulting build dir contained only buildroot's own stamp files, no repo
# content, and `cargo install --path` failed with "does not contain a
# Cargo.toml". Overriding Build/Prepare with a direct copy is the
# unambiguous, verified-working alternative for a local-only source.
OW_REPO_ROOT:=/mnt/f/luci-app-overwatch
PKG_BUILD_DIR:=$(BUILD_DIR)/$(PKG_NAME)-$(PKG_VERSION)

PKG_MAINTAINER:=JohnsonRan
PKG_LICENSE:=MIT

PKG_BUILD_DEPENDS:=rust/host

include $(INCLUDE_DIR)/package.mk
# This package is reached through a symlink (package/overwatch/overwatchd ->
# this repo's openwrt/overwatchd/), and `make -C` follows symlinks to their
# REAL target directory before evaluating the Makefile — so a relative
# include here resolves against this repo's real path on disk, not the SDK's
# package/ tree, no matter how many "../" a feed-relative convention like
# dufs's "../../lang/rust/rust-package.mk" would use. $(TOPDIR) (already used
# above for rules.mk) is the one path OpenWrt guarantees absolute and correct
# regardless of how this Makefile was reached.
include $(TOPDIR)/feeds/packages/lang/rust/rust-package.mk

# Copy the repo into PKG_BUILD_DIR directly (see the OW_REPO_ROOT comment
# above for why `PKG_SOURCE_PROTO:=file` doesn't do this on its own).
# tar-pipe rather than rsync/cp -a: no extra host-tool dependency, and it
# naturally preserves the workspace's file structure (Cargo.toml/.lock, src/)
# that `cargo install --path` needs.
define Build/Prepare
	mkdir -p $(PKG_BUILD_DIR)
	tar --exclude=.git --exclude=target --exclude=openwrt \
		-cf - -C $(OW_REPO_ROOT) . | tar -xf - -C $(PKG_BUILD_DIR)
endef

define Package/overwatchd
  SECTION:=net
  CATEGORY:=Network
  TITLE:=Per-client traffic monitor
  URL:=https://github.com/JohnsonRan/luci-app-overwatch
  DEPENDS:=$(RUST_ARCH_DEPENDS)
endef

define Package/overwatchd/description
  overwatchd monitors per-client traffic (conntrack-based, WED/hardware-
  offload safe).
endef

define Package/overwatchd/conffiles
/etc/config/overwatch
endef

define Package/overwatchd/install
	$(INSTALL_DIR) $(1)/usr/bin
	$(INSTALL_BIN) $(PKG_INSTALL_DIR)/bin/overwatchd $(1)/usr/bin/overwatchd

	$(INSTALL_DIR) $(1)/etc/config
	$(INSTALL_CONF) $(CURDIR)/../../files/etc/config/overwatch $(1)/etc/config/overwatch

	$(INSTALL_DIR) $(1)/etc/init.d
	$(INSTALL_BIN) $(CURDIR)/files/overwatchd.init $(1)/etc/init.d/overwatchd

	$(INSTALL_DIR) $(1)/etc/overwatch
endef

$(eval $(call RustBinPackage,overwatchd))
$(eval $(call BuildPackage,overwatchd))
