################################################################################
# rod — builds the Rust binary from the repo root via cargo-package
################################################################################

ROD_VERSION = local
# Source is the repository root, two levels up from this external tree.
ROD_SITE = $(BR2_EXTERNAL_ROD_PATH)/..
ROD_SITE_METHOD = local
ROD_LICENSE = MIT

# Runtime deps: D-Bus (bluer crate) + BlueZ (bluetoothd peer)
ROD_DEPENDENCIES = host-rustc dbus bluez5_utils

# Strip the release binary — saves ~10 MB.
ROD_CARGO_ENV = RUSTFLAGS="-C strip=symbols"

# cargo-package's default BUILD_CMDS passes --offline, relying on crates
# having been vendored by its DOWNLOAD_POST_PROCESS=cargo hook — which only
# runs on an actual downloaded tarball/git checkout. SITE_METHOD=local never
# produces one, so that vendoring never happens and --offline just fails
# ("no matching package found") the moment any dependency isn't already
# sitting in the shared cargo registry cache from some unrelated package.
# Override BUILD_CMDS ourselves, dropping --offline, since this is our own
# first-party crate (no need for Buildroot's vendored-source reproducibility
# guarantees, which are aimed at third-party packages) and the runner has
# normal network access.
define ROD_BUILD_CMDS
	cd $(@D) && \
	$(TARGET_MAKE_ENV) \
		$(TARGET_CONFIGURE_OPTS) \
		$(PKG_CARGO_ENV) \
		$(ROD_CARGO_ENV) \
		cargo build \
			--release \
			--manifest-path Cargo.toml \
			--locked
endef

define ROD_INSTALL_TARGET_CMDS
	$(INSTALL) -D -m 0755 \
		$(@D)/target/$(RUSTC_TARGET_NAME)/release/rod \
		$(TARGET_DIR)/usr/bin/rod
endef

$(eval $(cargo-package))
