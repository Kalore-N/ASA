# 使用项目根目录的静态 Linux 二进制构建。
# 构建命令（在项目根目录执行）：
#   docker build -f deploy/Dockerfile -t asa-relay .
FROM alpine:3.20

COPY dist/asa /usr/local/bin/asa
RUN chmod +x /usr/local/bin/asa

EXPOSE 9000

ENTRYPOINT ["/usr/local/bin/asa", "relay", "serve", "9000"]
