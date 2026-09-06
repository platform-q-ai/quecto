import json, subprocess, sys
for data, count in [("",0),("a\n",1),("a\nb",2),("\n\n",2)]:
    for args in ([], ["--json"]):
        r = subprocess.run([sys.executable,"-B","linecount.py"] + args,
                           input=data,text=True,capture_output=True,timeout=10)
        assert r.returncode == 0 and r.stderr == "", r
        assert r.stdout.endswith("\n")
        if args:
            obj = json.loads(r.stdout)
            assert obj == {"lines":count} and type(obj["lines"]) is int
        else:
            assert r.stdout == f"lines={count}\n"
help_result = subprocess.run([sys.executable,"-B","linecount.py","--help"],capture_output=True,text=True,timeout=10)
assert help_result.returncode == 0 and "--json" in help_result.stdout
print("8 CLI mode/input cases and help pass")
