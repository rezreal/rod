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

define ROD_INSTALL_TARGET_CMDS
	$(INSTALL) -D -m 0755 \
		$(@D)/target/$(RUSTC_TARGET_NAME)/release/rod \
		$(TARGET_DIR)/usr/bin/rod
endef

$(eval $(cargo-package))
