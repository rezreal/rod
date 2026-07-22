################################################################################
# rpi-bt-firmware — BCM Bluetooth HCD firmware files for RPi 3/4/5
################################################################################

# Pinned to a commit, not the "master" branch name: RPi-Distro/bluez-firmware
# renamed its default branch to pios/trixie, and Buildroot's git downloader's
# incremental fetch couldn't resolve "master" from its cached mirror even
# though the branch still exists upstream. A commit SHA sidesteps branch
# resolution entirely and pins the firmware version for good.
ROD_RPI_BT_FIRMWARE_VERSION = 6851250bf9d51ff50e3c5b2cf2111e2419d4335b
ROD_RPI_BT_FIRMWARE_SITE = https://github.com/RPi-Distro/bluez-firmware
ROD_RPI_BT_FIRMWARE_SITE_METHOD = git
ROD_RPI_BT_FIRMWARE_LICENSE = Proprietary
ROD_RPI_BT_FIRMWARE_LICENSE_FILES = broadcom/BCM-LEGAL.txt
ROD_RPI_BT_FIRMWARE_REDISTRIBUTE = NO

define ROD_RPI_BT_FIRMWARE_INSTALL_TARGET_CMDS
	$(INSTALL) -d $(TARGET_DIR)/lib/firmware/brcm
	$(INSTALL) -m 0644 $(@D)/broadcom/*.hcd $(TARGET_DIR)/lib/firmware/brcm/
endef

$(eval $(generic-package))
