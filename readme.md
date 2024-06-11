**Read this in other languages: [English](README.md), [中文](README_zh.md).**

**Process Compose** is a lightweight alternative to docker-compose, but instead of orchestrating containers, it orchestrates processes. It uses a YAML configuration syntax similar to docker-compose to configure and manage processes, mainly for deploying microservices applications in resource-sensitive environments and quickly setting up microservices application environments in development and testing environments.

## Background
With the current trend of microservices architecture, a business application is often composed of multiple independent services. Through container orchestration tools like K8S, Docker Compose, and DevOPS tools like Jenkins, we can easily deploy services to containers and deploy them to production environments through configuration and orchestration. However, these tools, while convenient, are relatively heavy, and the resource consumption of container orchestration tools like K8S is unacceptable for certain lightweight environments. While solutions like K3S provide a lighter alternative, in scenarios with particularly strict resource limitations, virtualized containers themselves introduce additional overhead.

Additionally, tools like K8S and Jenkins require a high level of operational capability. If one wishes to deploy and debug projects with microservices architecture in a development environment, it often means manually deploying multiple service containers and managing their states, significantly increasing the burden on developers during the debugging phase.

Therefore, in the era of microservices, even with so many convenient tools and platforms available, I feel there is still a need for a more convenient service management tool that caters to lightweight and development environments.

## Features
In essence, the services we typically start in microservices are processes of the operating system. Therefore, orchestrating and managing services in a non-virtualized environment on a single machine can be simplified to orchestrating and managing processes. **Process Compose** is precisely such a tool. It can manage and monitor the lifecycle of processes, as well as their dependencies, treating an application composed of a series of services (or processes) as a whole to start, stop, monitor, etc. Its core features include:

1. Process Monitoring:
   **Process Compose** can self-register as a system service of the current operating system to monitor managed processes, similar to what supervisor does, but it can monitor multiple processes simultaneously and configure different process health check methods (check methods, check intervals, etc.).
   
2. Process Orchestration:
   **Process Compose** can specify dependencies between managed services, similar to what most container orchestration tools do. This allows for controlling the startup sequence of different services, solving issues like the need for the database to start before the application service.

3. Process Lifecycle Management:
   **Process Compose** can start, stop, restart, etc., managed processes as a whole. Once the application's relevant parameters are configured, subsequent control of the application's start and stop can be done through simple commands, which is very convenient for developing and debugging microservices applications.

## Usage
The usage of Process Compose is straightforward. You can assemble and execute your services in just a few steps:
1. First, download an executable file of **Process Compose**. Binary packages for common environments are provided in the GitHub repository. If the deployment environment you need is not provided, compile the source code yourself.
   
2. Prepare your service's startup files and related configurations, placing them in the same directory as Process Compose, with each service in a subdirectory, like:
```bash
	process-compose directory
			|-- service1
					|- configuration and executable files for this service
			|-- service2
					|- configuration and executable files for this service
			|--  ......
			|--  config.yaml    #configuration file for process-compose
```

3. Write a config.yaml configuration file specifying information about the managed services. Below is a template:
```yaml
log_level: info # Log level
app_data_home: D://tmp//process-compose//home # Data directory for managed services, default is the .process-compose folder in the home directory of the current user
sys_service_name: process-compose # Service name registered as a system service
sys_service_desc: Process Monitoring and Management Tool # Description of the service registered as a system service
services:
    # Configuration of managed services, multiple services can be configured below
    service1: # Service name
      # Whether to redirect the log output of the startup command to a specific file as the service log (generally used in scenarios where the service cannot actively output log files), the redirected log will be placed in the {app_data_home}/{service_name}/logs directory
      log_redirect: false
      healthcheck: 
        test_type: http  # Supports four types: http, cmd, tcp, process. The default is process, which checks whether the process is alive
        test_target: http://localhost:23800/api/demo/test  # The test target is determined based on the value of test_type. For http, the complete URL starting with http:// needs to be configured; for tcp, the IP:port needs to be configured; for cmd, the command to be executed needs to be configured
        timeout: 5      # Timeout for health check, in seconds
        interval: 10    # Interval for health check, in seconds
        retries: 3      # Number of failed health checks to determine service failure
        start_period: 2 # Initialization time required after the service starts, during this period health checks will not be performed
      # Startup command, the path to . is adjusted here to point to the main directory of the service itself
      # For example, if the process-compose executable is located in the /home/nobody/app directory
      # Then the actual path of ./runtime/bin/java is /home/nobody/app/service1/runtime/bin/java
      start_cmd: ["./runtime/bin/java", "-jar","test.jar"] 
    service2:
      log_redirect: true 
      healthcheck:
        interval: 10 
        retries: 1    
        start_period: 5
      # Actual startup path is {directory where process-compose is located}/service2/test
      start_cmd: ["./test"]
      # Names of other services it depends on, services with dependencies configured will wait for the dependent services to have OK health status before starting
      depends_on:
        - service1
```

4. Execute relevant commands of Process Compose for service installation, startup, etc.:
```bash
process-compose #start process-compose and its managed services without using system services
process-compose install  #register process-compose as a system service
process-compose start    #start services registered through install
process-compose stop     #stop services
```

## Operating System Support
Windows: Windows 7 and above versions,
Linux: Supports mainstream distributions with systemd.