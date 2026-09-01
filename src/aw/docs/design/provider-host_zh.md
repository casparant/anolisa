# Headless Provider Host

[English](provider-host.md)

Provider Host PoC 通过 Headless 的有界调用验证一个有版本的系统
Capability。首个 Driver 是 `exec-json/v1`：一次受控进程只处理一次调用。

## 边界

Core 与 Host 之间交换 `CapabilityInvocation` 和
`ProviderInvocationResult`。已有 Provider binary 继续使用自己的原生 JSON
协议；Host 根据 Capability 声明的 `json-map/v1` codec，把规范输入映射成
原生 stdin，再把原生 stdout 映射回规范结果。映射逻辑不根据
`provider_id` 写特例。

```text
规范 CapabilityInvocation
        |
        | 校验身份、作用域、digest、deadline 和 budget
        v
json-map/v1 请求映射 ---> 原生 JSON ---> exec-json Provider
                                             |
瞬态规范输出 <---------- json-map/v1 <--- 原生 JSON
无正文 Receipt <--------- disposition、meters、evidence 事实
```

输入、输出 Contract 都引用内容寻址的 JSON 资源。准入时要求资源为严格相对
路径、普通文件，SHA-256 与原始字节一致，并且内容是合法 JSON。这个 PoC
尚不负责完整的 JSON Schema 实例校验。

Payload digest 使用 **Agent Workload canonical JSON v1**：递归按 key 排序 Object，
保留 Array 顺序，并输出紧凑 UTF-8 JSON。写入 `exec-json/v1` stdin 的也是
同一组字节。

## 结果语义

- `Produced` 表示 Advise Provider 生成了一个候选结果，不代表 Core 已经
  采纳或交付。
- 只有 `Produced` 携带瞬态 output；可持久化 Receipt 只记录 output 的
  schema、digest 和编码后字节数。
- 调用被接受之后发生 timeout、非零退出、输出超限、非法 JSON 或映射失败，
  都返回不含正文的 `Failed` Receipt。
- Manifest、schema、作用域、digest、deadline、budget 和 state root 路径
  校验失败发生在调用接受之前，直接返回准入错误；已准入的 state directory
  若无法创建，则返回不含正文的 `Failed` Receipt。
- 当前 one-shot PoC 支持 ExecutionContext 到 ToolCall 作用域上的 Observe、
  Advise 和 Mediate Capability。在副作用 reconcile 与相应调用作用域落地前，
  Enforce、Host 和 User 声明不会通过准入。

## Package 声明

Manifest 声明 Provider 对网络、环境变量、文件系统和数据处理的需求。
Runtime Capability Graph 会展示这些声明，并明确标为
`declared_not_enforced`。当前 Host 会清空继承环境并限制时间和 I/O，但 PoC
没有声称已用 OS sandbox 强制网络和文件系统声明。

## Headless 命令

这些命令属于开发与诊断面。该 binary 尚不作为公开 CLI 打包或安装，
其语法也不是兼容性 Contract。

所有发现路径都必须显式给出绝对路径。Manifest 目录使用
`<root>/<provider-id>/provider.toml` 布局，包目录名必须与 Manifest 中的
Provider 身份一致。裸 executable 名只会在显式 `--executable-root` 下解析，
Host 不扫描 `PATH`。

```console
$ aw-provider-host --output jsonl list \
    --manifest /opt/agent-workload/providers/tokenless/provider.toml \
    --executable-root /opt/agent-workload/bin

$ aw-provider-host --output jsonl doctor \
    --manifest-dir /opt/agent-workload/providers \
    --executable-root /opt/agent-workload/bin

$ aw-provider-host --output jsonl invoke \
    --manifest /opt/agent-workload/providers/tokenless/provider.toml \
    --executable-root /opt/agent-workload/bin \
    --invocation-file /tmp/context-projection-invocation.json
```

只有 Manifest 引用了 `{provider_state_dir}` 时才需要 `--state-root`；无状态
Provider 不会收到或创建 state directory。

invoke 响应会同时返回瞬态 `outcome` 和无正文 `receipt`。通用 ledger 可以
保存 receipt，但不能因此顺带保存 outcome。

## 非目标

这个 PoC 不创建 Agent Work，不调度 Agent runtime，不持久化 Receipt，不
reconcile 外部副作用，不对 executable 字节做证明或锁定，不替换 Provider
的原生协议，不发现任意 executable，也不宣称已经完成全部权限强制。它会
携带调用方提供的 idempotency 与 policy 元数据，但不负责请求去重或 policy
判定。这些分别属于 Core 和 sandbox 的后续职责。
