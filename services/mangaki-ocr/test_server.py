import sys
import threading
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
from server import OcrRuntime, _remove_nested_furigana


def block(text, x, y, width, height):
    return {
        "text": text,
        "tightBoundingBox": {"x": x, "y": y, "width": width, "height": height},
        "forcedOrientation": "vertical",
    }


class FuriganaFilterTests(unittest.TestCase):
    def test_removes_small_kana_region_nested_in_main_text(self):
        main = block("証明はできる", 0.1, 0.1, 0.4, 0.5)
        ruby = block("しょうめい", 0.38, 0.2, 0.1, 0.15)
        self.assertEqual(_remove_nested_furigana([main, ruby]), [main])

    def test_keeps_independent_kana_only_bubble(self):
        main = block("証明はできる", 0.1, 0.1, 0.35, 0.4)
        kana = block("できない", 0.65, 0.1, 0.2, 0.35)
        self.assertEqual(_remove_nested_furigana([main, kana]), [main, kana])


class QueueLimitTests(unittest.TestCase):
    def test_rejects_work_beyond_bounded_pending_queue(self):
        runtime = OcrRuntime.__new__(OcrRuntime)
        runtime._request_slots = threading.BoundedSemaphore(3)
        self.assertTrue(runtime.enter_queue())
        self.assertTrue(runtime.enter_queue())
        self.assertTrue(runtime.enter_queue())
        self.assertFalse(runtime.enter_queue())
        runtime.leave_queue()
        self.assertTrue(runtime.enter_queue())


if __name__ == "__main__":
    unittest.main()
