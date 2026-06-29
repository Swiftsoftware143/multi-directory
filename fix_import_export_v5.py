p = "/opt/swift/multidirectory-rust/src/handlers/import_export.rs"
c = open(p).read()
# Fix: replace v.replace('"', chr(39) '""' chr(39))
# with proper string version
c = c.replace("v.replace('\u{0022}', '\u{0022}\u{0022}')", 'v.replace("\u{0022}", "\u{0022}\u{0022}")')
open(p, "w").write(c)
print("Fixed")