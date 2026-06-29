content = open("/opt/swift/multidirectory-rust/src/handlers/import_export.rs").read()
# Bad: v.replace('\''"'\'', '\''""'\'')
# Fix: v.replace("\"", "\"\"")
content = content.replace(
    '''v.replace('"', '""')''',
    '''v.replace("\"", "\"\"")'''
)
open("/opt/swift/multidirectory-rust/src/handlers/import_export.rs", "w").write(content)
print("Fixed")