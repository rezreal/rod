################################################################################
# rpi-bt-firmware — BCM Bluetooth HCD firmware files for RPi 3/4/5
################################################################################

ROD_RPI_BT_FIRMWARE_VERSION = master
ROD_RPI_BT_FIRMWARE_SITE = https://github.com/RPi-Distro/bluez-firmware
ROD_RPI_BT_FIRMWARE_SITE_METHOD = git
ROD_RPI_BT_FIRMWARE_LICENSE = Proprietary
ROD_RPI_BT_FIRMWARE_LICENSE_FILES = LICENCE.broadcom_bcm43xx
ROD_RPI_BT_FIRMWARE_REDISTRIBUTE = NO

define ROD_RPI_BT_FIRMWARE_INSTALL_TARGET_CMDS
	$(INSTALL) -d $(TARGET_DIR)/lib/firmware/brcm
	$(INSTALL) -m 0644 $(@D)/broadcom/*.hcd $(TARGET_DIR)/lib/firmware/brcm/
endef

$(eval $(generic-package))
