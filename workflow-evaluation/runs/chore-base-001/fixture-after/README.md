# Item export
Export item records as a CSV file for spreadsheet users.

## Example
python3 export.py data/items.json --dest build/items.csv

## Options
source: JSON source file (required positional argument)
--dest: CSV destination file (required)
--delimiter: column delimiter (default comma)

## Compatibility
Consumers rely on the id,name column order.

Runtime: Python 3.10+ standard library; no installation or network needed.
