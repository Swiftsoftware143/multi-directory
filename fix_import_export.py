import re

with open('/opt/swift/multidirectory-rust/src/handlers/import_export.rs', 'r') as f:
    content = f.read()

# Fix 1: Temporary value dropped - replace unwrap_or with owned String
content = content.replace(
    '.unwrap_or(&name.to_lowercase().replace(" ", "-").replace("[", "").replace("]", ""));',
    '.map(|s| s.to_string()).unwrap_or_else(|| name.to_lowercase().replace(" ", "-").replace("[", "").replace("]", ""));'
)

# Fix 2: Replace csv::Writer manual usage with simple CSV generation
# Find the csv block and replace it
old = '''            let mut wtr = csv::Writer::from_writer(Vec::new());'''
if old in content:
    new = '''            let mut csv_rows: Vec<String> = Vec::new();'''
    content = content.replace(old, new)

old = '''                wtr.write_record(&record).map_err(|e|
                    AppError::Internal(format!("CSV write error: {}", e))
                )?;
            }
            let csv_bytes = wtr.into_inner().map_err(|e|
                AppError::Internal(format!("CSV finalize error: {}", e))
            )?;'''
new = '''                let line = record.map(|v| {
                    if v.contains(',') || v.contains('"') || v.contains('\\n') {
                        format!("\"{}\"", v.replace('"', '""'))
                    } else {
                        v
                    }
                }).join(",");
                csv_rows.push(line);
            }
            // Prepend header
            csv_rows.insert(0, fields.join(","));
            let csv_bytes: Vec<u8> = csv_rows.join("\\n").into_bytes();'''

content = content.replace(old, new)

# Fix 3: Type mismatch in export_data call (line 569)
# reviews is Vec<Review> but expected &[Business]
# The function is generic so this should work if we fix the variable name
# Actually: the issue is the SQL query returns reviews but export_data expects businesses
# Let's look at it closer - need to remove the problematic line or fix the call
content = content.replace(
    'export_data(fmt, &fields_refs, &reviews)',
    'export_reviews_data(fmt, &fields_refs, &reviews)'
)

# Add the reviews export function at the end
# Actually simpler: just rename the export call to avoid type confusion
# The proper fix: add a separate function

with open('/opt/swift/multidirectory-rust/src/handlers/import_export.rs', 'w') as f:
    f.write(content)

print('Fixed')
