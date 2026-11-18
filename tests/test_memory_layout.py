"""检查 Flash 分区一致性，并使用实际链接脚本验证越界会构建失败。"""

import re
import subprocess
import tempfile
import tomllib
import unittest
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]


class MemoryLayoutTests(unittest.TestCase):
    def setUp(self):
        memory = (ROOT / "memory.x").read_text()
        flash = re.search(
            r"FLASH\s*:\s*ORIGIN\s*=\s*(0x[0-9a-fA-F]+),\s*LENGTH\s*=\s*(\d+)K",
            memory,
        )
        self.assertIsNotNone(flash, "无法识别 memory.x 的 FLASH 区域")
        self.origin = int(flash[1], 16)
        self.length = int(flash[2]) * 1024

    def test_firmware_ends_at_existing_storage_partition(self):
        storage = tomllib.loads((ROOT / "keyboard.toml").read_text())["storage"]
        self.assertEqual(storage.get("start_addr"), 0xA0000)
        self.assertEqual(storage.get("num_sectors"), 32)
        self.assertEqual(self.origin, 0x1000)
        self.assertEqual(self.origin + self.length, storage["start_addr"])
        self.assertEqual(storage["start_addr"] + storage["num_sectors"] * 4096, 0xC0000)

    def test_linker_accepts_boundary_and_rejects_one_byte_overflow(self):
        with tempfile.TemporaryDirectory() as directory:
            work = Path(directory)
            sections = work / "sections.x"
            sections.write_text("SECTIONS { .text : { *(.text) } > FLASH }\n")
            for size, should_fit in [(self.length, True), (self.length + 1, False)]:
                with self.subTest(size=size):
                    source = work / "image.s"
                    source.write_text(f'.section .text,"ax"\n.space {size}\n')
                    obj = work / "image.o"
                    subprocess.run(
                        ["arm-none-eabi-as", "-mcpu=cortex-m4", "-o", str(obj), str(source)],
                        check=True,
                        capture_output=True,
                        text=True,
                    )
                    result = subprocess.run(
                        [
                            "arm-none-eabi-ld", "-T", str(ROOT / "memory.x"),
                            "-T", str(sections), "-o", str(work / "image.elf"), str(obj),
                        ],
                        capture_output=True,
                        text=True,
                    )
                    self.assertEqual(result.returncode == 0, should_fit, result.stderr)
                    if not should_fit:
                        self.assertIn("FLASH", result.stderr)


if __name__ == "__main__":
    unittest.main()
