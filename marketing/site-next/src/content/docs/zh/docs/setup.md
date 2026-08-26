---
title: "建立一个连接"
description: "从点 ＋ 到拿到一个能敲命令的 shell，中间发生的全部事情：表单里每个字段、 密钥从哪来、指纹提示怎么读、multiplexer 怎么选，以及主机上没装 tmux / herdr / mosh-server 时 Moshpit 会怎么做。主机现成的话，一分钟左右。"
---

## 加主机

「名称」「主机」「端口」「用户名」。只要「名称」和「主机」都有内容，「保存」 就能点了，其余都有能用的默认值。

![Add Connection 表单：Name 填 review-demo，Host 填 demo.moshpit.cluas.eu.org，Port 填 2222，Username 填 review，Authentication 选中 Password 那个 tab](/34-add-connection.jpg)

## 选认证方式

密码、在手机上生成的密钥，或者粘一段 PEM。密钥本身进 iOS Keychain， 连接记录里只留一个指向它的引用。

## 核对指纹

第一次握手会停下来，把主机公钥指纹和用来比对的 `ssh-keygen` 命令一起摆给你。信任，或者取消。

## 先说一件小事：界面是中英混排的

绝大多数标签有中文，但 herdr 相关的几处 （`Multiplexer`、`None`、`Custom herdr Path`、 `Install herdr`）以及 `Copy Public Key` 等几个还没翻译， 中文系统下仍然显示英文。另外，凡是文案里要按 multiplexer 换词的地方 （空态、「创建会话 / 创建 Workspace」按钮、附加中的提示），目前一律走英文—— 换词是靠拼接实现的，拼出来的句子还没进翻译表。下面按你在屏幕上**实际看到的样子**写。
