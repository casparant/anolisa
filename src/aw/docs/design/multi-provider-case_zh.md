# 多 Provider 案例

[English](multi-provider-case.md)

这是一次端到端记录：三个 Capability，由两个 Provider 在两个 Agent 边界上提供服务，每一个裁决都落进同一条可验证的 Ledger 链。它的存在是为了回答一个具体问题：当同时有多个 Provider、多个 authority 参与时，Provider 抽象还站得住吗？

## 案例覆盖范围

| 边界 | Capability | Authority | Provider | 选路 |
| --- | --- | --- | --- | --- |
| PreToolUse | `security.command.inspect/v1` | Mediate | agent-sec-core | 恰好一个 |
| PostToolUse | `security.content.inspect/v1` | Observe | agent-sec-core | 所有不同 Provider |
| PostToolUse | `security.code.inspect/v1` | Observe | agent-sec-core | 所有不同 Provider |
| PostToolUse | `context.projection.prepare/v1` | Advise | tokenless | 恰好一个 |

有两件事是单 Provider 路径验证不到的：

- **一个计划里两个 Provider。** PostToolUse 计划扇出到 agent-sec-core 两次、tokenless 一次，顺序固定，三次调用全部记录在同一行 Ledger 里。
- **三种 authority，三套失败策略。** Observe 步骤记录 gap 并继续；Advise 步骤拒绝整个计划；Mediate 步骤应用中介默认值。路由中没有任何地方按 `provider_id` 选择行为。

Observe 先于 Advise 是刻意的。关于原始 artifact 的事实在任何派生表示存在之前就被记录，因此数据流已经朝着未来「不要压缩含有秘密的内容」这类策略所需的方向运行。

## 验证范围 —— 请先读这一节

下面这次运行在 Linux 上执行（`5.10.134-011.ali5000.al8.x86_64`，cargo 1.94.1），实际经过了：

- 两个真实 Provider manifest，未做修改，包含摘要校验过的 contract
- 真实的 Provider admission、`json-map/v1` codec 映射与进程管控
- 真实的 Core 路由、计划顺序与失败策略
- 真实的 Ledger admission、哈希链、SQLite 存储与验证

**叶子可执行文件是 shim，不是真实 Provider。** `agent-sec-cli` 需要 `uv` 和 Python 3.11.6，验证机上都没有，而在共享主机上安装工具链超出了本次范围。shim 说的是与真实 Provider 相同的原生协议，其响应也通过同一份 `native_output` schema 校验。

因此这验证了边界的 AW 一侧端到端可用。它**不**验证 agent-sec-core 自身的检测规则或 tokenless 自身的压缩效果 —— 那是各组件自己的职责，有各自的测试。

## 复现步骤

```bash
cargo build --manifest-path src/aw/Cargo.toml \
  -p aw-cosh-hook --bin aw-cosh-hook \
  -p aw-ledger --bin aw-ledger

V=$(mktemp -d); mkdir -p "$V/bin" "$V/ledger"
```

两个说 Provider 原生协议的 shim：

```bash
cat > "$V/bin/agent-sec-cli" <<'SH'
#!/bin/sh
payload=$(cat)
case "$payload" in
  *command_inspect*) printf '%s' '{"protocol_version":1,"disposition":"completed","verdict":"deny","reasons":["shim.recursive_delete"],"findings":[{"rule_id":"shim.recursive_delete","category":"dangerous_pattern","severity":"critical","confidence":"high","count":1}],"findings_total":1,"scanned_bytes":9,"truncated":false}' ;;
  *code_inspect*)    printf '%s' '{"protocol_version":1,"disposition":"completed","verdict":"suspicious","findings":[{"rule_id":"shim.download_exec","category":"dangerous_pattern","severity":"medium","confidence":"high","count":1}],"findings_total":1,"scanned_bytes":48,"truncated":false,"language_detected":"bash"}' ;;
  *)                 printf '%s' '{"protocol_version":1,"disposition":"completed","verdict":"sensitive","findings":[{"rule_id":"shim.aliyun_ak","category":"secret","severity":"high","confidence":"high","count":1}],"findings_total":1,"scanned_bytes":48,"truncated":false}' ;;
esac
SH

cat > "$V/bin/tokenless" <<'SH'
#!/bin/sh
cat > /dev/null
printf '%s' '{"protocol_version":1,"output":"builds[2] compressed","disposition":"applied","reversibility":"lossless","before_tokens":120,"after_tokens":30,"tokenizer_id":"shim-v1","compressor_chain":["shim"]}'
SH

chmod +x "$V/bin/agent-sec-cli" "$V/bin/tokenless"
```

两个边界，一个 Ledger，`required` 保证级别：

```bash
HOOK=src/aw/target/debug/aw-cosh-hook
COMMON="--manifest-dir $PWD/providers --executable-root $V/bin
        --target-id case-host --allow-unenforced-provider
        --provider-wall-time-ms 30000
        --ledger $V/ledger --ledger-mode required"

$HOOK --event PreToolUse  $COMMON < pre.json
$HOOK --event PostToolUse $COMMON < post.json
```

`--manifest-dir` 会发现 `providers/<provider-id>/provider.toml` 形式的包。Host 只接受显式绝对根路径，从不搜索环境里的 `PATH`。

## 实测结果

### 1. PreToolUse —— 闸门阻断

输入命令：`rm -rf / --no-preserve-root`。

```json
{"decision":"block","reason":"AW · security · blocked · shim.recursive_delete"}
```

原因里带的是规则码，不是命令。`SecurityRuleId` 被限制在 `[a-z0-9._-]`，所以闸门提示在结构上就不可能回显它拒绝的那个参数。

### 2. PostToolUse —— 三次调用，两个 Provider

输入的工具输出里含一个阿里云 access key 和一个 pipe-to-shell。

```json
{"suppressOutput":true,
 "systemMessage":"AW · tokenless · estimated context 120→30 tokens · saved 75%\nAW · security · 2 findings · peak high",
 "hookSpecificOutput":{"hookEventName":"PostToolUse","updatedToolResponse":"builds[2] compressed"}}
```

一个响应同时报告两种 authority：tokenless 的投影和 agent-sec-core 的发现。摘要只计数不点名 —— 与拒绝不同，一次观察不需要在提示层面就可行动。

### 3. 链可验证

```console
$ aw-ledger --ledger "$V/ledger" verify
verified 2 record(s); chain intact

$ aw-ledger --ledger "$V/ledger" list
     0  evt_f2b22530-...  pre_tool_use_gate   aw.ledger.pre_tool_use_gate/v1   8af7ed2a...  tool_use=tol_6666...
     1  evt_39f41a62-...  post_tool_use_plan  aw.ledger.post_tool_use_plan/v1  8d26d4a7...  tool_use=tol_6666...
2 record(s)
```

两个边界落进同一条链、同一个 `tool_use_id` 之下。记录 1 的 `parent.digest` 等于记录 0 的 `record_digest` —— 这是 `verify` 重算出来的，不是读出来的。

### 4. plan 记录容纳完整的多 Provider 轨迹

`aw-ledger body evt_39f41a62-...`，格式化后：

```json
{
  "source_artifact_id": "art_77eb2165-...",
  "source_digest": "54cf4c59387e1c67...",
  "observations": [
    { "capability": {"id": "security.content.inspect", "version": 1},
      "verdict": "sensitive",
      "findings": [{"rule_id":"shim.aliyun_ak","category":"secret","severity":"high","confidence":"high","count":1}],
      "scanned_bytes": 48, "truncated": false,
      "invocation": {"provider_id":"agent-sec-core","provider_version":"0.11.0",
                     "invocation_id":"pvi_ab9a0e47-...","disposition":"produced",
                     "output_digest":"b138bb236db9734f..."} },
    { "capability": {"id": "security.code.inspect", "version": 1},
      "verdict": "suspicious",
      "findings": [{"rule_id":"shim.download_exec","category":"dangerous_pattern","severity":"medium","confidence":"high","count":1}],
      "language_detected": "bash", "scanned_bytes": 48, "truncated": false,
      "invocation": {"provider_id":"agent-sec-core","provider_version":"0.11.0",
                     "invocation_id":"pvi_d551d7f4-...","disposition":"produced",
                     "output_digest":"e38353b71aa7e324..."} }
  ],
  "observation_gaps": [],
  "projection": {
    "candidate_offered": true, "media_type": "text/plain",
    "reversibility": "lossless", "transform_chain": ["shim"],
    "invocation": {"provider_id":"tokenless","provider_version":"0.7.14",
                   "invocation_id":"pvi_2cf7bf72-...","disposition":"produced",
                   "output_digest":"f63c8554639274a5..."}
  }
}
```

每个断言都可归因：每条事实都指明产生它的 Capability、服务它的 Provider 与版本，以及支撑它的那次调用。这里 `observation_gaps` 为空；若某个扫描器缺失或失败，它会写明 Capability 和原因，读者才能区分「什么都没找到」和「没人去看」。

### 5. 敏感内容没有进入数据库

对每一段流经系统的内容，直接探查原始数据库文件：

```console
$ for n in 'LTAI5tSecretValue' 'rm -rf' 'no-preserve-root' \
           'builds[2] compressed' 'curl http'; do
    strings "$V/ledger/ledger.db" | grep -qF -- "$n" \
      && echo "LEAK: [$n]" || echo "OK: [$n] absent"
  done
OK: [LTAI5tSecretValue] absent
OK: [rm -rf] absent
OK: [no-preserve-root] absent
OK: [builds[2] compressed] absent
OK: [curl http] absent
```

五个探针：扫描器找到的秘密、闸门拒绝的命令（两段）、Advise 步骤产出的投影候选、代码扫描器标记的危险模式。它们在文件里任何位置都不出现 —— 不在数据行里，不在 WAL 页里，也不在空闲页残留里。

探针用 `strings` 读文件，刻意绕开 SQL。查询只能显示 schema 暴露的东西；这个方式显示物理上真实存在的东西。

## 这确立了什么

- 在一个计划里同时有两个 Provider、三种 authority 时，Provider 抽象成立，且计划本身没有点名任何 Provider。
- Observe 扇出触达每一个不同的 Provider；Mediate 与 Advise 只接纳一个，而这个差异来自计划策略，不是特例分支。
- 两个边界写进同一条哈希链，读者仅凭存储字节即可重算。
- 内容自由在真实的多 Provider 运行中成立，而且是对着数据库文件验证的，不是从记录模型推断的。

## 这没有确立什么

- 真实 `agent-sec-core` 检测或真实 `tokenless` 压缩 —— 叶子可执行文件是 shim，见上文**验证范围**。
- 并发写入行为。两次 hook 调用是串行的。过渡期写入方的竞争问题在
  [AW Ledger](ledger_zh.md#过渡期的-hook-侧写入方) 中做了论证，但未在此测量。
- Provider 沙箱。必须加 `--allow-unenforced-provider`，正因为声明的网络与文件系统权限只被校验、尚未由 OS 沙箱强制。

记录模型与哈希链见 [AW Ledger](ledger_zh.md)；Advise 数据模型见
[上下文投影](context-projection_zh.md)。
