p = "/opt/swift/multidirectory-rust/src/handlers/import_export.rs"
c = open(p).read()
# ONLY $N -> \\x24N, nothing else
import re
c = re.sub(r"\$(\d+)", r"\\x24\1", c)
open(p, "w").write(c)
print("Fixed $N")