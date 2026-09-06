import argparse
import sys
def main():
    parser = argparse.ArgumentParser()
    parser.parse_args()
    count = sum(1 for _ in sys.stdin)
    print(f"lines={count}")
if __name__ == "__main__":
    main()
