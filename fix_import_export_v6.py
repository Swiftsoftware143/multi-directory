p = "/opt/swift/multidirectory-rust/src/handlers/import_export.rs"
c = open(p).read()
# Use chr(39) for single quote, chr(34) for double quote
sq = chr(39)  # single quote
dq = chr(34)  # double quote
bad = "v.replace(" + sq + dq + sq + ", " + sq + dq + dq + sq + ")"
good = "v.replace(" + dq + "\\\\" + dq + dq + ", " + dq + "\\\\" + dq + "\\\\" + dq + ")"
c = c.replace(bad, good)
open(p, "w").write(c)
print("Fixed")