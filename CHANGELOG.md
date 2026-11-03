# v0.2.1

- C2S 输入上限由 2 MiB 放宽至 20 MiB，CLI、库解析和 HTTP API 使用相同上限；其他资源限制保持不变。
- 增加 20 MiB 边界、超限拒绝和 JSON 转义膨胀的真实 API 回归验证。

# v0.2.0

- 新增 JSON 服务配置和 `init-config`，可指定直接存放 C2S 的目录，并生成随机、持久的 ETag 盐。
- 新增内容和渲染参数绑定的 HMAC-SHA-256 ETag；匹配 `If-None-Match` 时返回无正文的 304，跳过渲染。兼容 Chunibot `newchart` 的 `name`、`judge`、`div` 参数。
- 新增 `/api/render/1086_03`、`/fastattack/1086_03` 和 `fastattack=1`。Fast ATTACK 模式保留全部音符，只显示仍有提前 ATTACK 的音符的提前 ATTACK / JUSTICE / JC。
- 修正地面音符的相邻保护：原始时间差中点、FIX 下限、分类 MINIMALIZE 和同刻 EXT 覆盖。内部 JC 重叠保留，显示使用不累积叠色的浅浮雕。
- 加固文件读取、符号链接 / 重解析点边界、请求参数、错误信息和子进程限制；升级 Tokio 安全补丁。库接口 `service::router` 现在返回 `Result`，`Options` 新增 `early_attack_only` 与 `divisions`。
- 提供 Windows x64 可执行文件（静态 C 运行库）、Linux x64 musl 静态可执行文件和 SHA-256 校验文件。双平台测试及依赖审计通过后才发布。

依赖审计仍报告 `ttf-parser` 停止维护；它只读取程序内嵌的固定字体，不接受外部字体。详见 `SECURITY.md`。
