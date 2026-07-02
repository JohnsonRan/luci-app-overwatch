include $(TOPDIR)/rules.mk

PKG_NAME:=luci-app-overwatch
PKG_VERSION:=0.1.0
PKG_RELEASE:=1

PKG_LICENSE:=MIT
PKG_MAINTAINER:=JohnsonRan

LUCI_TITLE:=Overwatch — per-client traffic monitor UI
LUCI_DESCRIPTION:=LuCI frontend for overwatchd: per-client traffic dashboard and history charts.
LUCI_URL:=https://github.com/JohnsonRan/luci-app-overwatch
LUCI_MAINTAINER:=JohnsonRan
LUCI_DEPENDS:=+overwatchd +rpcd +luci-base +socat
LUCI_SUBMENU_FORCED:=3. Applications

# Repo deliberately ships unminified/vendored files (Chart.js UMD bundles
# etc) — keep luci.mk's Build/Prepare (copies this package's own htdocs/
# and root/ into PKG_BUILD_DIR, then installs them) and its po/-driven
# luci-i18n-overwatch-<lang> sub-package generation, but skip its
# JsMin/CssTidy passes so vendored bundles aren't rewritten.
LUCI_MINIFY_JS:=0
LUCI_MINIFY_CSS:=0

# Content lives under this package's own htdocs/ and root/ (luci.mk
# convention), NOT the repo-level files/ tree — files/etc/config/overwatch
# is owned by overwatchd (a conffile there); this package must not also
# claim it, or the two packages (always co-installed, DEPENDS +overwatchd)
# would conflict on the same path.
include $(TOPDIR)/feeds/luci/luci.mk

# call BuildPackage - OpenWrt buildroot signature
# (grep-only marker: include/scan.mk's package scanner greps each package's
# OWN Makefile text for this literal string — it does not follow includes —
# so without this line the scanner silently drops this package from the
# build entirely, even though luci.mk calls BuildPackage for real at its
# own tail. Confirmed against feeds/luci/applications/luci-app-nlbwmon/Makefile,
# which carries the identical trailing comment for the same reason.)
