import re
p = "/opt/swift/multidirectory-rust/src/handlers/import_export.rs"
with open(p) as f:
    c = f.read()
# ONLY replace $N with \\x24N, nothing else
c = re.sub(r"\$(\d+)", r"\\x24\1", c)
with open(p, "w") as f:
    f.write(c)
print("Done - fixed $N -> \\x24N")