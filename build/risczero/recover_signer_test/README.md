# Recover Signer Test Guest Program

这个guest program用于在zkvm环境中复现`recover_signer`失败的问题。

## 问题描述

在生成derive proof时，zkvm运行fpvm时，在`crates/kona/src/kona.rs:299`调用`recover_signer`会失败。虽然创建了一个test case (`crates/kona/src/kona.rs:542-633`)尝试复现这个问题，但test case里调用`recover_signer`不会失败。test case的数据来源于`crates/kona/src/kona.rs:307-320`失败时打印的程序。

## 使用方法

### 1. 构建guest program

首先需要构建guest program。在项目根目录运行：

```bash
cargo build -F rebuild-recover-signer-test
```

### 2. 运行测试

运行CLI命令来执行测试：

```bash
cargo run --bin kailua-cli -- recover-signer-test
```

或者如果已经安装了kailua-cli：

```bash
kailua-cli recover-signer-test
```

## 测试数据

测试使用的transaction数据来自失败的场景：

- nonce: 242
- gas_limit: 21000
- gas_price: 20900000000
- value: 32370000000000000
- chain_id: Some(11155111)
- to: 0x359a68f67966247a34e07694493e0d00c99a1756
- input: empty
- signature_r: 111197453629367114907912549862485227720359187220219358471218136821626017544888
- signature_s: 16069675716490115033286433543232569847835186933082730357946014073768762936666
- y_parity: false

## 预期结果

如果成功复现问题，guest program会输出错误信息，说明`recover_signer`失败的原因。
