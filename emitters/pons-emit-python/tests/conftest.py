import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
TESTS = Path(__file__).resolve().parent

for entry in (ROOT, TESTS):
    text = str(entry)
    if text not in sys.path:
        sys.path.insert(0, text)
