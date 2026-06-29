import re

p = "/opt/swift/multidirectory-rust/src/handlers/import_export.rs"
with open(p) as f:
    c = f.read()

# Fix 1: $N -> \\x24N (already done but confirm)
c = re.sub(r"\$(\d+)", r"\\x24\1", c)

# Fix 2: v.replace('""', '""""') -> use string params instead of char
c = c.replace("""v.replace('"', "\"\"")""", """v.replace("\\"", "\\"\\"")""")

# Fix 3: &businesses -> &contacts in export_contacts function
c = c.replace("serde_json::to_value(&businesses)", "serde_json::to_value(&contacts)")

# Fix 4: Missing record function - fix export_data csv parse
# Change: record.map(|v| { -> record.into_iter().map(|v| {
c = c.replace("record.map(|v| {", "record.into_iter().map(|v| {")

# Fix 5: Add generics to export_data function
c = c.replace("fn export_data(", "fn export_data<T>(")

# Fix 6: Fix format string - format!(""{}"", -> format!() with proper syntax
# Actually the format!(""{}"", ...) is fine in Rust; "" inside format macros = literal "
# The real issue is v.replace('"', "\"\"") which uses char '' with 2 chars
# Fix 6 already covered by fix 2

with open(p, "w") as f:
    f.write(c)
print("Comprehensive fix applied")

# Verify
with open(p) as f:
    c = f.read()
# Check no more bad patterns
if "v.replace('" in c and "'" in c:
    print("WARNING: still has char replace pattern")
if "&businesses)" in c:
    print("WARNING: still has &businesses")
if "record.map" in c:
    print("WARNING: still has record.map")