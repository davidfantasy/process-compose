## 开发背景
当前微服务架构大行其道，一个业务应用往往是由多个独立的服务构成，通过K8S，Docker Compose等容器编排工具，以及Jekins等DevOPS工具，我们也能够很方便的将服务发布到容器，并通过配置和编排轻松部署到线上环境。但这些工具虽然方便，但是比较重，对于某些轻量化环境，K8S等容器编排工具的资源消耗是无法接受的。当然，我们也可以选择K3S这样更轻量的解决方案，但在资源限制特别苛刻的场景下，虚拟化容器本身就会带来一些额外的消耗。

同时K8S，Jekins等都对运维能力有比较高的要求，如果希望在开发环境自行部署和调试微服务架构的项目，那往往意味着必须手动部署多个服务的容器并管理它们的状态，这也会大大加剧开发人员在开发阶段进行调试的负担。

所以在微服务时代，即使我们已经拥有了这么多方便的工具和平台，我感觉仍然需要一种更加方便的，服务于轻量化和开发环境的服务管理工具。
## 功能介绍
其实本质上，我们通常的微服务启动后都是操作系统的进程，所以对服务的编排和管理，在单机的非虚拟化环境下实际上可以简化为对进程的编排和管理。**Process Compose**正是这样的一款工具。它能够管理和监控进程的生命周期，以及相互的依赖关系，将由一系列服务（或者说进程）组成的应用当作一个整体去启动，停止，监控等。它的核心功能有三点：

 1. 进程监控：
	**Process Compose**可以自注册为当前操作系统的系统服务，对受管进程进行监控，类似于supervisor所做的事情，但能够同时监控多个进程，而且能够配置不同进程的健康检查方式（检查方式，检查间隔等）
 2. 进程编排：
 	**Process Compose**可以指定受管服务之间的依赖关系。正如大多数容器编排工具所做的那样。这样就能控制不同服务的启动顺序，这样就能解决比如数据库需要先于应用服务进行启动的问题
 3. 进程生命周期管理：
 	**Process Compose** 能够将受管进程作为一个整体去启动，停止，重启等。只要配置好应用的相关参数，后续就能通过简单的命令来控制应用的启停，这对于微服务应用的开发调试非常方便。
 ## 使用方式
 Process Compose的使用方式非常简单，你只需要几步就能够将你的服务组装起来并执行：
1. 你需要首先下载一个 **Process Compose** 的可执行文件，github仓库中提供了常见环境的二进制包的下载。如果你需要部署的环境没有提供，请自行编译源码。
 2.  准备好你自己的服务的启动文件和相关配置，可以将它们都放到Process Compose的同级目录，每个服务一个子目录，类似于：
 

```bash
	process-compose所在目录
			|-- service1
					|- 该服务的配置和可执行文件等
			|-- service2
					|- 该服务的配置和可执行文件等
			|--  ......
			|--  config.yaml    #process-compose的配置文件
```
     						
 3.  编写一个config.yaml的配置文件，指定所管服务的信息，配置文件的项目如下：
```yaml
log_level: info #日志等级
app_data_home: D://tmp//process-compose//home #所管服务的数据目录，默认是当前用户的主目录的.process-compose文件夹
sys_service_name: process-compose # 注册为系统服务的服务名
sys_service_desc: Process Monitoring and Management Tool #注册为系统服务的服务描述
services:
    #以下为受管服务配置，可配置多个服务
    service1: #服务名
      #启动延迟，秒；配置后会在轮到该服务启动时延迟执行启动命令，一般用于等待上游依赖服务启动完成
      startup_delay: 5
      #是否要重定向启动命令的日志输出到特定文件，作为服务日志（一般用于服务无法主动输出日志文件的场景），重定向的日志会放到{app_data_home}/{service_name}/logs目录下
      log_redirect: false
      healthcheck: 
        enable: true    #是否启用健康检查
        restart: true   #服务失败后是否尝试重启服务
        url: http://localhost:23800/api/demo/test  #如果是web服务，可以配置健康检查接口，接口返回200认为是成功；默认仅检查进程是否仍在运行
        interval: 10    #健康检查的间隔，秒
        retries: 3      #健康检查失败多少次判断服务失败
        check_delay: 5 #启动后，多少秒后才开始第一次健康检查
      #启动命令，这里对.的路径做了调整，指向了服务自己的主目录
      #例如 process-compose可执行文件放到了/home/nobody/app目录下
      #那么./runtime/bin/java实际的路径是/home/nobody/app/service1/runtime/bin/java
      start_cmd: ["./runtime/bin/java", "-jar","test.jar"] 
      depends_on:
        - service2  #所依赖的其它服务名
    service2:
      log_redirect: true 
      healthcheck:
        enable: true
        restart: true 
        interval: 10 
        retries: 1    
        start_period: 5
      #实际启动路径为{process-compose所在目录}/service2/test
      start_cmd: ["./test"]
```
4. 执行process-compose的相关命令，进行服务的安装，启动等操作：

```bash
process-compose #不通过系统服务直接启动process-compose及其受管服务
process-compose install  #将process-compose注册为系统服务
process-compose start    #启动通过install注册的服务
process-compose stop     #停止服务
```