//! Template engine module — Multi-Directory Handlebar rendering.
//!
//! Loads and renders Handlebar templates for different directory types.
//! Templates are embedded at compile time via `include_str!`.

use handlebars::Handlebars;
use serde_json::Value;

// ── Template constants ──────────────────────────────────────────────────────

pub const TEMPLATE_LOCAL_BUSINESS: &str = "local-business";
pub const TEMPLATE_FARM: &str = "farm";
pub const TEMPLATE_RESTAURANT: &str = "restaurant";
pub const TEMPLATE_REAL_ESTATE: &str = "real-estate";
pub const TEMPLATE_MEDICAL: &str = "medical";
pub const TEMPLATE_SERVICE: &str = "service";

const VALID_TEMPLATES: [&str; 6] = [
    TEMPLATE_LOCAL_BUSINESS,
    TEMPLATE_FARM,
    TEMPLATE_RESTAURANT,
    TEMPLATE_REAL_ESTATE,
    TEMPLATE_MEDICAL,
    TEMPLATE_SERVICE,
];

// ── Compile-time embedded template strings ──────────────────────────────────
// These are used when .hbs files aren't found at runtime.
// They are embedded at compile time via include_str! directives.
// The paths are relative to this source file: src/template_engine.rs
// templates/ directory is at project root, so relative path is ../templates/

/// Check if a template ID is valid
pub fn is_valid_template(template_id: &str) -> bool {
    VALID_TEMPLATES.contains(&template_id)
}

/// Get all available template IDs and their display names
pub fn get_available_templates() -> Vec<TemplateInfo> {
    vec![
        TemplateInfo { id: TEMPLATE_LOCAL_BUSINESS.to_string(), name: "Local Business".to_string(), description: "Standard business directory with search, categories, and ratings".to_string() },
        TemplateInfo { id: TEMPLATE_FARM.to_string(), name: "Farm / Farmers Market".to_string(), description: "Farmers market, produce, pick-your-own, seasonal listings".to_string() },
        TemplateInfo { id: TEMPLATE_RESTAURANT.to_string(), name: "Restaurant".to_string(), description: "Restaurant directory with menus, hours, delivery info".to_string() },
        TemplateInfo { id: TEMPLATE_REAL_ESTATE.to_string(), name: "Real Estate".to_string(), description: "Property listings with agents, open houses, maps".to_string() },
        TemplateInfo { id: TEMPLATE_MEDICAL.to_string(), name: "Medical / Healthcare".to_string(), description: "Medical providers, specialties, insurance accepted".to_string() },
        TemplateInfo { id: TEMPLATE_SERVICE.to_string(), name: "Service Professionals".to_string(), description: "Plumbers, electricians, contractors with service areas".to_string() },
    ]
}

/// Default color scheme for directories
pub fn default_color_scheme() -> Value {
    serde_json::json!({
        "primary": "#2563eb",
        "secondary": "#64748b",
        "accent": "#f59e0b",
        "background": "#ffffff",
        "text": "#1e293b",
        "heading": "#0f172a",
        "muted": "#94a3b8",
        "border": "#e2e8f0"
    })
}

/// Normalize a color_scheme JSON value — fill in missing keys with defaults
pub fn normalize_color_scheme(scheme: Option<Value>) -> Value {
    let defaults = default_color_scheme();
    match scheme {
        Some(Value::Object(map)) => {
            let mut merged = defaults.as_object().unwrap().clone();
            for (k, v) in map {
                merged.insert(k, v);
            }
            Value::Object(merged)
        }
        _ => defaults,
    }
}

/// Information about an available template
#[derive(Debug, Clone, serde::Serialize)]
pub struct TemplateInfo {
    pub id: String,
    pub name: String,
    pub description: String,
}

// ── TemplateEngine ──────────────────────────────────────────────────────────

/// The template engine wrapper around the handlebars registry.
pub struct TemplateEngine {
    registry: Handlebars<'static>,
}

impl TemplateEngine {
    /// Create a new template engine and register all embedded templates.
    pub fn new() -> Self {
        let mut registry = Handlebars::new();
        registry.set_strict_mode(false);
        // Allow raw HTML in templates (don't escape &, <, >, etc.)
        registry.register_escape_fn(handlebars::no_escape);
        
        // Register all 6 templates
        let templates = [
            (TEMPLATE_LOCAL_BUSINESS, "directory-local-business.hbs"),
            (TEMPLATE_FARM, "directory-farm.hbs"),
            (TEMPLATE_RESTAURANT, "directory-restaurant.hbs"),
            (TEMPLATE_REAL_ESTATE, "directory-real-estate.hbs"),
            (TEMPLATE_MEDICAL, "directory-medical.hbs"),
            (TEMPLATE_SERVICE, "directory-service.hbs"),
        ];

        for (id, _filename) in &templates {
            let content = match *id {
                TEMPLATE_LOCAL_BUSINESS => include_str!("../templates/directory-local-business.hbs"),
                TEMPLATE_FARM => include_str!("../templates/directory-farm.hbs"),
                TEMPLATE_RESTAURANT => include_str!("../templates/directory-restaurant.hbs"),
                TEMPLATE_REAL_ESTATE => include_str!("../templates/directory-real-estate.hbs"),
                TEMPLATE_MEDICAL => include_str!("../templates/directory-medical.hbs"),
                TEMPLATE_SERVICE => include_str!("../templates/directory-service.hbs"),
                _ => include_str!("../templates/directory-local-business.hbs"),
            };
            if let Err(e) = registry.register_template_string(id, content) {
                eprintln!("Warning: failed to register template '{}': {}", id, e);
            }
        }

        Self { registry }
    }

    /// Load template from embedded content (called in new())
    pub fn load_all(&mut self) {
        // Templates are already loaded in new() via include_str!
        // This method exists for API compatibility
    }

    /// Register a template manually (only needed for runtime-loaded templates)
    pub fn register_template(&mut self, name: &str, content: &str) -> Result<(), String> {
        self.registry
            .register_template_string(name, content)
            .map_err(|e| format!("Failed to register template '{}': {}", name, e))
    }

    /// Render a directory page with the given template.
    ///
    /// # Arguments
    /// * `template_id` - The template identifier (e.g., "local-business", "restaurant")
    /// * `data` - A JSON value containing all template variables
    ///
    /// # Returns
    /// Rendered HTML string, or an error message as a string.
    pub fn render_directory_page(&self, template_id: &str, data: &Value) -> Result<String, String> {
        // Validate template ID, fallback to local-business
        let tid = if is_valid_template(template_id) {
            template_id
        } else {
            TEMPLATE_LOCAL_BUSINESS
        };

        self.registry
            .render(tid, data)
            .map_err(|e| format!("Render error for template '{}': {}", tid, e))
    }

    /// Get a list of all available template names
    pub fn available_templates(&self) -> Vec<String> {
        let template_ids: Vec<String> = VALID_TEMPLATES.iter().map(|&s| s.to_string()).collect();
        template_ids
    }
}

impl Default for TemplateEngine {
    fn default() -> Self {
        Self::new()
    }
}

// ── Helper: prepare template data from directory + businesses ───────────────

/// Build a JSON context object for template rendering.
///
/// Expected inputs:
/// - `directory`: serialized directory info (name, description, slug, etc.)
/// - `businesses`: array of business objects
/// - `categories`: array of category objects
/// - `color_scheme`: optional color scheme (will be normalized)
/// - `query`: optional search query parameters
pub fn build_template_context(
    directory: &Value,
    businesses: &Value,
    categories: &Value,
    color_scheme: Option<Value>,
    query: Option<Value>,
) -> Value {
    let colors = normalize_color_scheme(color_scheme);
    let q = query.unwrap_or_else(|| serde_json::json!({}));

    serde_json::json!({
        "directory": directory,
        "businesses": businesses,
        "categories": categories,
        "colors": colors,
        "query": q,
    })
}

// ── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_valid_templates() {
        assert!(is_valid_template(TEMPLATE_LOCAL_BUSINESS));
        assert!(is_valid_template(TEMPLATE_FARM));
        assert!(is_valid_template(TEMPLATE_RESTAURANT));
        assert!(is_valid_template(TEMPLATE_REAL_ESTATE));
        assert!(is_valid_template(TEMPLATE_MEDICAL));
        assert!(is_valid_template(TEMPLATE_SERVICE));
        assert!(!is_valid_template("nonexistent"));
    }

    #[test]
    fn test_default_color_scheme() {
        let scheme = default_color_scheme();
        assert!(scheme.get("primary").is_some());
        assert!(scheme.get("secondary").is_some());
        assert!(scheme.get("accent").is_some());
        assert_eq!(scheme["primary"], "#2563eb");
    }

    #[test]
    fn test_normalize_color_scheme() {
        let partial = serde_json::json!({"primary": "#ff0000"});
        let normalized = normalize_color_scheme(Some(partial));
        assert_eq!(normalized["primary"], "#ff0000");
        assert_eq!(normalized["secondary"], "#64748b"); // from defaults
    }

    #[test]
    fn test_get_template_content() {
        let engine = TemplateEngine::new();
        let data = serde_json::json!({
            "directory": {"name": "Test", "description": "test", "slug": "test"},
            "businesses": [],
            "categories": [],
            "colors": super::default_color_scheme(),
            "query": {}
        });
        let result = engine.render_directory_page(TEMPLATE_LOCAL_BUSINESS, &data);
        assert!(result.is_ok());
        let html = result.unwrap();
        assert!(html.contains("<!DOCTYPE html>"));
    }

    #[test]
    fn test_template_engine_new() {
        let engine = TemplateEngine::new();
        let templates = engine.available_templates();
        assert_eq!(templates.len(), 6);
    }

    #[test]
    fn test_render_local_business() {
        let engine = TemplateEngine::new();
        let data = serde_json::json!({
            "directory": {
                "name": "Test Directory",
                "description": "A test",
                "slug": "test-dir"
            },
            "businesses": [],
            "categories": [],
            "colors": default_color_scheme(),
            "query": {}
        });
        let result = engine.render_directory_page(TEMPLATE_LOCAL_BUSINESS, &data);
        assert!(result.is_ok());
        let html = result.unwrap();
        assert!(html.contains("Test Directory"));
        assert!(html.contains("A test"));
    }

    #[test]
    fn test_render_with_businesses() {
        let engine = TemplateEngine::new();
        let data = serde_json::json!({
            "directory": {
                "name": "BizDir",
                "description": "A directory",
                "slug": "biz-dir"
            },
            "businesses": [
                {
                    "name": "Acme Corp",
                    "slug": "acme",
                    "description": "We do things",
                    "city": "Springfield",
                    "state": "IL",
                    "phone": "+15551234567",
                    "rating": 4.5,
                    "review_count": 12
                }
            ],
            "categories": [],
            "colors": default_color_scheme(),
            "query": {}
        });
        let result = engine.render_directory_page(TEMPLATE_LOCAL_BUSINESS, &data);
        assert!(result.is_ok());
        let html = result.unwrap();
        assert!(html.contains("Acme Corp"));
        assert!(html.contains("Springfield"));
    }

    #[test]
    fn test_unknown_template_falls_back() {
        let engine = TemplateEngine::new();
        let data = serde_json::json!({
            "directory": {"name": "Test", "description": "test", "slug": "test"},
            "businesses": [],
            "categories": [],
            "colors": default_color_scheme(),
            "query": {}
        });
        // Unknown template should use fallback
        let result = engine.render_directory_page("unknown-template", &data);
        assert!(result.is_ok());
    }
}
