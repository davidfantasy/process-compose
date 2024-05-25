use std::{collections::HashMap, fs::File, io::Read, sync::RwLock};

use anyhow::{Error, Result};
use serde::{Deserialize, Serialize};

use crate::{env, health::HealthCheckType};

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct GlobalConfig {
    #[serde(default = "default_log_level")]
    pub log_level: String,
    #[serde(default = "default_app_data_home")]
    pub app_data_home: String,
    #[serde(default = "default_sys_service_name")]
    pub sys_service_name: String,
    #[serde(default = "default_sys_service_desc")]
    pub sys_service_desc: String,
    pub services: HashMap<String, ServiceConfig>,
    pub api: Option<ApiConfig>,
}

fn default_log_level() -> String {
    "info".to_string()
}

fn default_app_data_home() -> String {
    let user_home = dirs::home_dir().unwrap_or(".".into());
    user_home
        .join(".process-compose")
        .to_str()
        .unwrap()
        .to_string()
}

fn default_sys_service_name() -> String {
    "process-compose".to_string()
}

fn default_sys_service_desc() -> String {
    "Process Monitoring and Management Tool".to_string()
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct ServiceConfig {
    #[serde(default = "default_service_name")]
    pub name: String,
    pub log_redirect: bool,
    pub log_pattern: Option<String>,
    pub healthcheck: Option<HealthCheckConfig>,
    pub start_cmd: Vec<String>,
    pub depends_on: Option<Vec<String>>,
}

fn default_service_name() -> String {
    "".to_string()
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct HealthCheckConfig {
    pub test_type: HealthCheckType,
    pub test_target: String,
    #[serde(default = "default_check_interval")]
    pub interval: i32,
    #[serde(default = "default_max_failures")]
    pub max_failures: i32,
    pub start_period: Option<i32>,
}

fn default_check_interval() -> i32 {
    5
}

fn default_max_failures() -> i32 {
    1
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct ApiConfig {
    pub enable: bool,
    pub host: String,
    pub port: String,
    pub username: String,
    pub password: String,
}

const CONFIG_FILE_NAME: &str = "config.yaml";
const MAX_DEPTH: i32 = 5;

static CONFIG: RwLock<Option<GlobalConfig>> = RwLock::new(None);

pub fn load_config() -> Result<GlobalConfig> {
    let mut config_file_path = env::ROOT_DIR.clone();
    config_file_path.push(CONFIG_FILE_NAME);
    let mut file = File::open(config_file_path)?;
    let mut contents = String::new();
    file.read_to_string(&mut contents)?;
    let mut config: GlobalConfig = serde_yaml::from_str(&contents)?;
    config.services.iter_mut().for_each(|(name, service)| {
        service.name = name.clone();
    });
    let mut config_global = CONFIG.write().unwrap();
    *config_global = Some(config.clone());
    Ok(config)
}

//仅用于测试
pub fn set_config(config: GlobalConfig) {
    CONFIG.write().unwrap().replace(config);
}

pub fn current_config() -> GlobalConfig {
    let config_global = CONFIG
        .read()
        .unwrap()
        .clone()
        .expect("config not initialized");
    config_global
}

pub fn find_service_config(name: &str) -> Option<ServiceConfig> {
    let config = current_config();
    config.services.get(name).cloned()
}

pub fn analyze_service_dependencies(services: &Vec<ServiceConfig>) -> Result<Vec<String>> {
    let mut result: Vec<String> = Vec::new();
    let mut remained: Vec<&ServiceConfig> = Vec::new();
    services.iter().for_each(|service| {
        if service.depends_on.is_none() {
            result.push(service.name.clone());
        } else {
            remained.push(service);
        }
    });
    if remained.len() == 0 {
        return Ok(result);
    }
    result = process_independent_service(&mut result, remained, 0)?;
    Ok(result)
}

fn process_independent_service(
    processed: &mut Vec<String>,
    remained: Vec<&ServiceConfig>,
    depth: i32,
) -> Result<Vec<String>> {
    if depth > MAX_DEPTH {
        return Err(Error::msg("The maximum recursion limit has been exceeded. There may be a circular dependency in the service configuration"));
    }
    let mut new_remained: Vec<&ServiceConfig> = Vec::new();
    remained.iter().for_each(|service| {
        let mut dep_solved = true;
        let depends = service.depends_on.clone().unwrap();
        depends.iter().for_each(|dep| {
            if !processed.contains(dep) {
                dep_solved = false;
            }
        });
        if dep_solved {
            processed.push(service.name.clone());
        } else {
            new_remained.push(service);
        }
    });
    if remained.len() == 0 {
        return Ok(processed.to_vec());
    }
    let depth = depth + 1;
    return process_independent_service(processed, new_remained, depth);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn create_service_config(name: &str, depends_on: Vec<&str>) -> ServiceConfig {
        ServiceConfig {
            name: name.to_string(),
            depends_on: Some(depends_on.iter().map(|&s| s.to_string()).collect()),
            log_redirect: false,
            log_pattern: None,
            healthcheck: None,
            start_cmd: vec!["".to_owned()],
        }
    }

    #[test]
    fn test_analyze_service_dependencies_no_dependencies() {
        let services = vec![
            create_service_config("service1", vec![]),
            create_service_config("service2", vec![]),
        ];
        let result = analyze_service_dependencies(&services).unwrap();
        assert_eq!(result, vec!["service1", "service2"]);
    }

    #[test]
    fn test_analyze_service_dependencies_with_dependencies() {
        let services = vec![
            create_service_config("service1", vec!["service2"]),
            create_service_config("service2", vec!["service3"]),
            create_service_config("service3", vec![]),
        ];
        let result = analyze_service_dependencies(&services).unwrap();
        assert_eq!(result, vec!["service3", "service2", "service1"]);
    }

    #[test]
    fn test_analyze_service_dependencies_with_circular_dependencies() {
        let services = vec![
            create_service_config("service1", vec!["service2"]),
            create_service_config("service2", vec!["service3"]),
            create_service_config("service3", vec!["service1"]),
        ];
        let result = analyze_service_dependencies(&services);
        assert!(result.is_err());
    }
}
