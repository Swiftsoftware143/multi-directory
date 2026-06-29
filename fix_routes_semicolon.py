p = "/opt/swift/multidirectory-rust/src/routes.rs"
c = open(p).read()
# Line 112 ends without semicolon, and line 113 starts a new route chain
# Fix: add ; to line 112, move line 113 to after the protected routes section
lines = c.split("\n")
# Find line with "phone-numbers/:id/provision" - it should have a ;
for i, line in enumerate(lines):
    if 'phone-numbers/:id/provision"' in line and 'post(call_tracking::provision_phone_number)' in line:
        # Add ; to this line if it doesn't have one
        stripped = line.rstrip()
        if not stripped.endswith(";"):
            lines[i] = stripped + ";"
        break
open(p, "w").write("\n".join(lines))
print("Fixed")