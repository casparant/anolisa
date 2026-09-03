# AW Ledger

[English](ledger.md)

Ledger 是 AW 在每个 Agent 边界上「决定了什么」的持久、可验证篡改的记录。它回答一个事后无法从别处得到答案的问题：*系统当时观察到什么、决定了什么，我能否证明这份记录没被改过？*

## 内容自由是设计约束，不是附加条款

Ledger 只存有界元数据、摘要和 ID。它从不存工具输出、命令文本、投影候选，也不存规则命中的那个值。

这不是事后补上的隐私修饰，而是塑造整个记录模型的约束。一份把自己所描述的内容抄一遍的审计日志，会变成第二个防护更弱的存储，里面正是 Observe Capability 被装上来寻找的那些秘密。一行写着 `shim.aliyun_ak, severity high, count 1` 的记录是可行动的；同时带上 `LTAI5t...` 的记录则是负债。

这一点由 admission 强制，而不是信任写入方。每个候选 body 都会被逐层遍历，任何对象携带以下键即被拒绝：

```text
command  tool_input  tool_response  matched  content  payload
```

检查大小写不敏感，且深入嵌套对象和数组元素。通过 admission 的 body 是被机器校验过的，不只是被人看过。

最尖锐的例子是 Advise 步骤。`ContextProjectionCandidate` 带有 `content: String` —— 就是模型可见的表示本身。因此 plan 记录只保留候选的摘要和有界的形状元数据：

```json
"projection": {
  "candidate_offered": true,
  "media_type": "text/plain",
  "reversibility": "lossless",
  "transform_chain": ["shim"],
  "invocation": { "output_digest": "f63c8554...", "...": "..." }
}
```

需要该表示的读者按摘要去 Artifact 存储取。Ledger 证明*当时指的是哪一个*候选，而不必成为它的副本。

## 记录模型

每条记录由 header 和 body 组成。header 本身是 schema，body 由该 schema 约束。

```text
LedgerRecordHeader
  id            evt_<uuid>        稳定记录标识
  sequence      u64               单调、无空洞
  timestamp_ms  u64               写入方挂钟时间
  kind          enum              事件分类
  schema        string            约束 `body` 的修订号
  parent        {id, digest}?     仅 sequence 0 时缺省
  body_digest   Digest            `body` 的 canonical JSON v1 摘要
```

`parent` 把前一条记录的标识**和**摘要绑在同一个结构里。若拆成两个可选字段，header 就可能引用一个 parent 却不承诺它的字节 —— 而这恰好是篡改者想要的状态。

目前有两个 body schema，对应 Core 的两个边界：

| Kind | Schema | 记录内容 |
| --- | --- | --- |
| `pre_tool_use_gate` | `aw.ledger.pre_tool_use_gate/v1` | 一次待执行 Tool Call 的 Mediate 闸门裁决 |
| `post_tool_use_plan` | `aw.ledger.post_tool_use_plan/v1` | 一次工具结果的完整 Observe + Advise 计划 |

分类里还有三个变体（`provider_invoked`、`evidence_stored`、`receipt_stored`）尚无写入方。声明它们是因为该枚举是增量的：日后追加变体不会让已存记录失效 —— 每条记录都已经声明了自己的 schema 修订号。

## 哈希链

每条记录承诺两次：对自己的 body，以及对前一条记录。

```text
记录 N-1                             记录 N
  body ──摘要──► body_digest          body ──摘要──► body_digest
  header+body ──摘要──► D(N-1)        header.parent.digest = D(N-1)
                                      header+body ──摘要──► D(N)
```

两个摘要都是 canonical JSON v1（键递归排序、紧凑分隔符、UTF-8）上的 SHA-256。确定性正是「可重算」有意义的前提：读者重新编码存储的值，必须得到相同字节。

`verify_chain` 按 sequence 顺序遍历，对每条记录：

1. 检查 sequence 与前一条相邻，
2. 检查 `parent` 与前一条的标识和摘要一致，
3. 从存储的 canonical body 字节重算 body 摘要，
4. 从存储的 canonical record 字节重算 record 摘要，
5. 解码 canonical record 字节，对其中内嵌的 body 重算摘要。

第 5 步最容易被省掉，而省掉的代价最大。第 3、4 步各自把一个存储摘要与紧挨它存放的字节比对，所以同时改写 `body_canonical` 与 `body_digest` 的攻击者能通过两者。第 5 步能抓到 —— 因为 `record_canonical` 里仍是原始 body。

代价与记录数线性相关。目前没有增量或检查点模式，非常大的 Ledger 会需要一个。

## 存储

SQLite，每个 Ledger 根目录一个文件，以 WAL 模式打开。记录存在 STRICT 表里，让数据库强制列类型，而不是信任写入方。

每次 append 在一个 `IMMEDIATE` 事务里同时插入记录行与 scope 行，且内存中的链尖只在提交成功后推进。因此失败的 append 让链停在原处，而不会留下一个指向无人存储的行的链尖。

`sequence` 带 `UNIQUE` 约束。admission 已经会拒绝非单调的 sequence，所以这是纵深防御：当两个写入方都认为自己拥有链尖时，起作用的正是这条约束。

Trace scope 放在按记录 ID 索引的侧表（`ledger_scope`），每个维度都有部分索引。把它移出记录表，意味着哈希链需要重算的列保持窄，而且日后新增 scope 索引不必触碰任何已提交字节。

> **读这个数据库需要 SQLite 3.37 或更新版本。** STRICT 表是在该版本引入的。系统自带更旧 `sqlite3` 的主机（包括其 Python 模块）会直接拒绝该 schema。请用 `aw-ledger` 可执行文件，它链接自带的 bundled SQLite。

## 有界查询

每条读路径都在带索引的列上过滤，返回量不超过索引选中的行。刻意不提供无界扫描。

| 访问器 | 使用的索引 |
| --- | --- |
| `record_by_id` | 主键 |
| `events_by_kind` | `idx_ledger_records_kind` |
| `events_for_attempt` | `idx_ledger_scope_attempt` |
| `record_body_bytes` | 主键 |

前三个返回 `StoredRecord`，携带 header、trace scope 和摘要 —— 但不含 body blob。取 body 是另一次显式调用。常见场景是过滤，而过滤不该把记录体读进内存。

## 过渡期的 hook 侧写入方

`aw-cosh-hook` 可以自己写记录，由 `--ledger` 和 `--ledger-mode` 控制。该模块被命名为*过渡期*，因为写入方本该在一个替整台机器持有数据库的守护进程里，而那个守护进程还不存在。

hook 侧写入方能承诺的是：两个并发 hook 进程在同一个 SQLite 文件上竞争，而 WAL 加上 `IMMEDIATE` 事务加上 `sequence` 的 `UNIQUE` 约束，意味着竞争的失败方**是 append 失败，而不是把链写坏**。这是安全的，但会丢记录。

`--ledger-mode` 是调用方表达「丢记录是否可接受」的方式：

| 模式 | append 失败时 |
| --- | --- |
| `correlated`（默认） | 边界照常放行。裁决仍然成立，只是 Ledger 不宣称它。 |
| `required` | 边界失败。在 PreToolUse 上，非零退出正是让 COSH 失败关闭的机制 —— 没被记录的闸门会阻断而不是放过。 |

`correlated` 不是静默兜底。`CoshHookRun.ledger_unavailable` 会声明「配置了写入方但它失败了」，这与「没配置写入方」是不同的事实；`ObservationGapReason` 和 `GateDegradation` 里已有的 `LedgerUnavailable` 变体就是为了在下游表达它。

append 在写出 hook 响应**之前**执行。反过来排序会让 `required` 失去意义：COSH 那时已经被告知该怎么做了。

## 检视一个 Ledger

`aw-ledger` 可执行文件是开发与诊断界面。它未被打包为公开 CLI，其命令语法也不是兼容性契约。

```bash
cargo build --manifest-path src/aw/Cargo.toml -p aw-ledger --bin aw-ledger
LEDGER=src/aw/target/debug/aw-ledger

# 重算每一个摘要和 parent 链接。
"$LEDGER" --ledger /path/to/ledger verify

# 每条记录一行：sequence、id、kind、schema、record 摘要、tool use。
"$LEDGER" --ledger /path/to/ledger list
"$LEDGER" --ledger /path/to/ledger list --kind post-tool-use-plan
"$LEDGER" --ledger /path/to/ledger list --attempt atm_<uuid>

# 恰好是被存进去的那些字节。
"$LEDGER" --ledger /path/to/ledger body evt_<uuid>
```

`body` 的存在是为了让内容自由可被审计，而不是被声称。它打印的就是 Ledger 持有的内容，运维可以直接 grep 不该在里面的东西，而不必相信「它不在里面」这句话。

`list` 不带过滤时会合并各 kind 的查询再按 sequence 重排。存储层不提供无界扫描，为一个便利视图加一个会破坏「查询有界」的初衷。

## Crate 边界

`aw-ledger` 只依赖 `aw-contracts`，且不得认识 Core 的 outcome 类型。

```text
aw-cosh-hook ──► aw-ledger ──► aw-contracts
      │                            ▲
      └──► aw-core ────────────────┘
```

记录 body schema 放在 `aw-contracts`，因为它们是有版本的 Contracts。从 Core outcome 到这些 body 的投影放在 `aw-core`，因为 Core 拥有 outcome —— `ToolResultOutcome::ledger_body` 和 `ToolCallDecision::ledger_body`。`aw-ledger` 是 `aw-core` 的 dev-dependency，正是为了让这些投影能对着真实 admission 被证明是内容自由的，而不是靠人看。

## 当前限制

- **没有守护进程写入方。** hook 进程就是写入方。并发 hook 在 `correlated` 下丢 append，在 `required` 下让边界失败。
- **没有增量验证。** `verify_chain` 与记录数线性相关。
- **没有保留期或压缩。** 记录无界累积。
- **没有签名。** 对拿到字节的读者来说链是可验证篡改的，但对能重写整个文件并重算每个摘要的人来说不是防篡改的。把链尖锚定到文件之外是后续工作。
- **三个分类变体没有写入方。** `provider_invoked`、`evidence_stored`、`receipt_stored` 已声明但未写入。

三个 Capability 跨两个 Provider 在两个边界上的端到端记录，见
[多 Provider 案例](multi-provider-case_zh.md)。
