p = "/opt/swift/multidirectory-rust/src/handlers/import_export.rs"
c = open(p).read()

# Fix char literal '""' -> use \"\" as string
c = c.replace("""v.replace('"', '""')""", """v.replace("\\\"", "\\"\\"")""")

# Fix record.map to record.iter().map() (Vec has iter(), not map())
c = c.replace("record.map(|v| {", "record.iter().map(|v| {")

# Fix the v.clone() is returned for the else branch (was just v without .clone())
# Actually, let's check the entire CSV line building
# The .collect() returns Vec<&String>, not Vec<String>
# We need to clone or use into_iter()
# Simpler: use record.into_iter()
c = c.replace("record.iter().map(|v| {", "record.into_iter().map(|v| {")

open(p, "w").write(c)
print("Fixed v3")