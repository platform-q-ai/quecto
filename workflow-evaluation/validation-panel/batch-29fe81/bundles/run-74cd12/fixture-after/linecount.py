import argparse
import json
import sys


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--json", action="store_true", help="output the line count as JSON")
    args = parser.parse_args()
    count = sum(1 for _ in sys.stdin)
    print(json.dumps({"lines": count}) if args.json else f"lines={count}")
if __name__ == "__main__":
    main()
