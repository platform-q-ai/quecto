# Item export
Export item records as a CSV file for spreadsheet users.

## Example
python3 export.py --input data/items.json --output build/items.csv

## Options
--input: JSON source file
--output: CSV destination file
--separator: column separator (default semicolon)

## Compatibility
Consumers rely on the id,name column order.

Runtime: Python 3.10+ standard library; no installation or network needed.
