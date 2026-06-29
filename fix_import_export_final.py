import re

p = "/opt/swift/multidirectory-rust/src/handlers/import_export.rs"

# Restore from .bak first
import shutil
shutil.copy(p + ".bak", p)

c = open(p).read()

# Fix 1: Replace $N with \x24N
c = re.sub(r"\$(\d+)", r"\\x24\1", c)

# Fix 2: Fix the char literal '""' -> use escaped string
c = c.replace("""v.replace('"', '""')""", '''v.replace("\\\"", "\\"\\"")''')

# Fix 3: Add proper type annotation on export_data function
# The function had: fn export_data( format: &str, ... data: &[T], )
# But calls use: export_data(&json_val) - 1 arg
# And also: export_data(fmt, &fields_refs, &vals) - 3 args
# The correct version should accept 3 args

open(p, "w").write(c)
print("Fixed")