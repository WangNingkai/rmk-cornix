MEMORY
{
  /* NOTE 1 K = 1 KiB = 1024 bytes */
  /* These values correspond to the nRF52840 WITH Adafruit nRF52 bootloader */
  /* 固件必须止于 0xA0000，后面的 128 KiB 留给 keyboard.toml 的配置存储。 */
  FLASH : ORIGIN = 0x00001000, LENGTH = 636K
  RAM : ORIGIN = 0x20000008, LENGTH = 255K

  /* These values correspond to the nRF52840 */
  /* FLASH : ORIGIN = 0x00000000, LENGTH = 1024K */
  /* RAM : ORIGIN = 0x20000000, LENGTH = 256K */
}
