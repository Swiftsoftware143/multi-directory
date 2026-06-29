import os, re

src_dir = "/opt/swift/multidirectory-rust/src"
d = chr(36)

for root, dirs, files in os.walk(src_dir):
    for fname in files:
        if not fname.endswith(".rs"):
            continue
        fpath = os.path.join(root, fname)
        with open(fpath, "r") as f:
            lines = f.readlines()
        modified = False
        new_lines = []
        for line in lines:
            if re.search(r"\$\d", line):
                new_line = re.sub(r"\$(\d+)", r"\\x24\1", line)
                if new_line != line:
                    modified = True
                    new_lines.append(new_line)
                else:
                    new_lines.append(line)
            else:
                new_lines.append(line)
        if modified:
            with open(fpath, "w") as f:
                f.writelines(new_lines)
            print(f"Fixed: {fpath}")

print("Verification...")
for root, dirs, files in os.walk(src_dir):
    for fname in files:
        if not fname.endswith(".rs"):
            continue
        fpath = os.path.join(root, fname)
        with open(fpath, "r") as f:
            c = f.read()
        for i, line in enumerate(c.split("\n"), 1):
            if d in line and re.search(r"\$\d", line.strip()) and not line.strip().startswith("//"):
                print(f"  STILL HAS $: {fpath}:{i}: {line.rstrip()[:120]}")