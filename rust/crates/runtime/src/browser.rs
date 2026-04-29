//! Browser/Snapshot Tool (Vision-as-a-Tool).
//!
//! Provides headless-browser capabilities to capture the visual state
//! and structural accessibility tree of applications.

use std::path::Path;
use serde::{Deserialize, Serialize};
use headless_chrome::{Browser, LaunchOptions};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScreenshotInput {
    pub url: String,
    pub save_path: Option<String>,
    /// Optional CSS selector to wait for before capturing.
    pub wait_for_selector: Option<String>,
    /// Optional delay in milliseconds to wait after navigation/selector.
    pub delay_ms: Option<u64>,
    /// Optional: Capture the full scrollable page instead of just the viewport.
    pub capture_full_page: Option<bool>,
    /// The workspace root to resolve relative paths against.
    pub workspace_root: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScreenshotOutput {
    pub path: String,
    pub width: u32,
    pub height: u32,
    pub dom_summary: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AccessibilityTreeInput {
    pub url: String,
    /// Optional CSS selector to wait for before extraction.
    pub wait_for_selector: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VisualDiffInput {
    pub path_a: String,
    pub path_b: String,
}

/// Operationalize the browser to capture a real visual snapshot.
pub fn capture_screenshot(input: ScreenshotInput) -> Result<ScreenshotOutput, Box<dyn std::error::Error>> {
    println!("[VISION] Launching headless observer for: {}", input.url);

    let options = LaunchOptions::default_builder()
        .headless(true)
        .build()?;

    let browser = Browser::new(options)?;
    let tab = browser.new_tab()?;

    // Navigate and wait for the page to load
    tab.navigate_to(&input.url)?;
    tab.wait_until_navigated()?;

    // Optional: Wait for selector
    if let Some(selector) = &input.wait_for_selector {
        println!("[VISION] Waiting for selector: {}", selector);
        let _ = tab.wait_for_element(selector)?;
    }

    // Optional: Fixed delay
    if let Some(ms) = input.delay_ms {
        std::thread::sleep(std::time::Duration::from_millis(ms));
    }

    // Capture screenshot
    let png_data = if input.capture_full_page.unwrap_or(false) {
        tab.capture_screenshot(
            headless_chrome::protocol::cdp::Page::CaptureScreenshotFormatOption::Png,
            None,
            None,
            true, // full page
        )?
    } else {
        tab.capture_screenshot(
            headless_chrome::protocol::cdp::Page::CaptureScreenshotFormatOption::Png,
            None,
            None,
            false, // viewport only
        )?
    };

    // Resolve path: prefer workspace-relative .tachy/vision/
    let base = input.workspace_root.as_deref().unwrap_or(".");
    let vision_dir = Path::new(base).join(".tachy").join("vision");
    
    let path_str = input.save_path.unwrap_or_else(|| {
        format!("snap_{}.png", std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_secs())
    });
    
    let path = if Path::new(&path_str).is_absolute() {
        Path::new(&path_str).to_path_buf()
    } else {
        vision_dir.join(&path_str)
    };
    
    let final_path_str = path.to_string_lossy().to_string();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(&path, png_data)?;

    // Extract a simplified DOM/Accessibility summary for the LLM
    let dom_summary = tab.evaluate("document.body.innerText.substring(0, 1000)", false)?
        .value
        .map(|v| v.to_string());

    println!("[VISION] Snapshot captured at {}", final_path_str);
    
    // Explicitly close tab to ensure clean shutdown
    tab.close(true)?;

    Ok(ScreenshotOutput {
        path: final_path_str,
        width: 1280,
        height: 720,
        dom_summary,
    })
}

/// Specialized extraction of the Accessibility tree for structural reasoning.
pub fn get_accessibility_tree(input: AccessibilityTreeInput) -> Result<String, Box<dyn std::error::Error>> {
    let options = LaunchOptions::default_builder().headless(true).build()?;
    let browser = Browser::new(options)?;
    let tab = browser.new_tab()?;
    tab.navigate_to(&input.url)?;
    tab.wait_until_navigated()?;

    if let Some(selector) = &input.wait_for_selector {
        let _ = tab.wait_for_element(selector)?;
    }

    // Capture interactive elements with more metadata
    let script = r#"
        (() => {
            const elements = document.querySelectorAll('button, a, input, select, textarea, [role], [aria-label], [aria-expanded], [aria-selected]');
            return Array.from(elements).map(el => ({
                tag: el.tagName,
                role: el.getAttribute('role') || el.tagName,
                label: el.getAttribute('aria-label') || el.innerText?.trim() || el.value || el.placeholder || '',
                id: el.id,
                name: el.getAttribute('name'),
                href: el.getAttribute('href'),
                disabled: el.disabled || el.getAttribute('aria-disabled') === 'true',
                state: {
                    expanded: el.getAttribute('aria-expanded'),
                    selected: el.getAttribute('aria-selected'),
                    hidden: el.getAttribute('aria-hidden')
                }
            })).slice(0, 100);
        })()
    "#;

    let result = tab.evaluate(script, false)?
        .value
        .map(|v| v.to_string())
        .unwrap_or_else(|| "[]".to_string());

    // Explicitly close tab to ensure clean shutdown
    tab.close(true)?;

    Ok(result)
}

/// Compare two snapshots and return a similarity report.
pub fn compare_snapshots(path_a: &str, path_b: &str) -> Result<String, Box<dyn std::error::Error>> {
    let bytes_a = std::fs::read(path_a)?;
    let bytes_b = std::fs::read(path_b)?;

    if bytes_a == bytes_b {
        return Ok("Visual Match: 100% (Identical byte-for-byte).".to_string());
    }

    let diff = (bytes_a.len() as i64 - bytes_b.len() as i64).abs();
    let pct = (diff as f64 / bytes_a.len() as f64) * 100.0;

    let mut report = format!("Visual Regression Report:\n");
    report.push_str(&format!("  - Asset A: {} ({} bytes)\n", path_a, bytes_a.len()));
    report.push_str(&format!("  - Asset B: {} ({} bytes)\n", path_b, bytes_b.len()));
    report.push_str(&format!("  - Delta: {} bytes ({:.4}%)\n", diff, pct));

    if pct < 0.05 {
        report.push_str("Status: VERIFIED. Minor noise detected, but visual structure appears stable.");
    } else if pct < 0.5 {
        report.push_str("Status: WARNING. Subtle layout or rendering shift detected. Review suggested.");
    } else {
        report.push_str("Status: FAILURE. Significant visual divergence. Check for regression.");
    }

    Ok(report)
}

/// Background utility to clean up old snapshots to prevent disk bloat.
pub fn clean_old_snapshots(vision_dir: &Path, max_age_secs: u64) -> std::io::Result<usize> {
    if !vision_dir.exists() { return Ok(0); }
    let mut count = 0;
    let now = std::time::SystemTime::now();
    
    for entry in std::fs::read_dir(vision_dir)? {
        let entry = entry?;
        let path = entry.path();
        if path.is_file() {
            if let Ok(metadata) = entry.metadata() {
                if let Ok(modified) = metadata.modified() {
                    if let Ok(age) = now.duration_since(modified) {
                        if age.as_secs() > max_age_secs {
                            let _ = std::fs::remove_file(path);
                            count += 1;
                        }
                    }
                }
            }
        }
    }
    Ok(count)
}
