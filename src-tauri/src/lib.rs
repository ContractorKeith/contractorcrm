use serde::Serialize;

/// Report returned by the `health` command — proves the UI → Rust seam works.
#[derive(Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct HealthReport {
    pub app: &'static str,
    pub version: &'static str,
    pub status: &'static str,
}

/// Pure health logic, kept separate from the Tauri command so it is testable.
fn health_report() -> HealthReport {
    HealthReport {
        app: "ContractorCRM",
        version: env!("CARGO_PKG_VERSION"),
        status: "ok",
    }
}

/// Health check command invoked from the UI over the application seam.
#[tauri::command]
fn health() -> HealthReport {
    health_report()
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .invoke_handler(tauri::generate_handler![health])
        .run(tauri::generate_context!())
        .expect("error while running ContractorCRM");
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::health_report;

    #[test]
    fn health_report_identifies_the_app_and_is_ok() {
        let report = health_report();
        assert_eq!(report.app, "ContractorCRM");
        assert_eq!(report.status, "ok");
        assert_eq!(report.version, env!("CARGO_PKG_VERSION"));
    }

    #[test]
    fn health_report_serializes_to_the_camel_case_wire_shape() {
        assert_eq!(
            serde_json::to_value(health_report()).expect("serialize health report"),
            json!({
                "app": "ContractorCRM",
                "version": env!("CARGO_PKG_VERSION"),
                "status": "ok"
            })
        );
    }
}
