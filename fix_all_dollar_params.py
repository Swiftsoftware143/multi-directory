import os, re

src_dir = "/opt/swift/multidirectory-rust/src"
count = 0

for root, dirs, files in os.walk(src_dir):
    for fname in files:
        if not fname.endswith(".rs"):
            continue
        fpath = os.path.join(root, fname)
        with open(fpath, "r") as f:
            content = f.read()
        new_content = re.sub(r"\$(\d+)", r"\\x24\1", content)
        if new_content != content:
            with open(fpath, "w") as f:
                f.write(new_content)
            count += 1
            print(f"Fixed: {fpath}")

print(f"\nFixed {count} files total")

# Debug: check what files still have $
for root, dirs, files in os.walk(src_dir):
    for fname in files:
        if not fname.endswith(".rs"):
            continue
        fpath = os.path.join(root, fname)
        with open(fpath, "r") as f:
            c = f.read()
        for i, line in enumerate(c.split("\n"), 1):
            if chr(36) in line and not line.strip().startswith("//"):
                print(f"  STILL HAS $: {fpath}:{i}")