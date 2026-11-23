#!/usr/bin/env bash
# Generates the certificates for the option tests (tests/broker/certs, not
# committed):
#   ca.crt / ca.key           test CA
#   server.crt / server.key   broker certificate for DNS:localhost only, so
#                             connecting via 127.0.0.1 fails the name check
#   client.crt / client.key   client certificate (mutual TLS)
#   client.pem                client certificate + key in one file
#   client-enc.key            client key, PKCS#8 encrypted with "secret"
#   other-ca.crt              an unrelated CA (must be refused)
#   capath/ca.pem             the CA alone in a directory (ssl.CApath)
set -euo pipefail
cd "$(dirname "$0")"
mkdir -p certs/capath
cd certs
export MSYS_NO_PATHCONV=1

openssl req -x509 -newkey rsa:2048 -nodes -keyout ca.key -out ca.crt -days 3650 \
  -subj "/CN=rsmqtt test CA" 2>/dev/null

openssl req -newkey rsa:2048 -nodes -keyout server.key -out server.csr -subj "/CN=localhost" 2>/dev/null
printf "subjectAltName=DNS:localhost\nbasicConstraints=CA:FALSE\nkeyUsage=digitalSignature,keyEncipherment\nextendedKeyUsage=serverAuth\n" > server.ext
openssl x509 -req -in server.csr -CA ca.crt -CAkey ca.key -CAcreateserial -out server.crt -days 3650 \
  -extfile server.ext 2>/dev/null

openssl req -newkey rsa:2048 -nodes -keyout client.key -out client.csr -subj "/CN=rsmqtt-client" 2>/dev/null
printf "basicConstraints=CA:FALSE\nkeyUsage=digitalSignature,keyEncipherment\nextendedKeyUsage=clientAuth\n" > client.ext
openssl x509 -req -in client.csr -CA ca.crt -CAkey ca.key -CAcreateserial -out client.crt -days 3650 \
  -extfile client.ext 2>/dev/null
cat client.crt client.key > client.pem
openssl pkcs8 -topk8 -in client.key -out client-enc.key -v2 aes-256-cbc -passout pass:secret 2>/dev/null

openssl req -x509 -newkey rsa:2048 -nodes -keyout other-ca.key -out other-ca.crt -days 3650 \
  -subj "/CN=some other CA" 2>/dev/null

cp ca.crt capath/ca.pem
rm -f ./*.csr ./*.ext ./*.srl
# mosquitto runs as an unprivileged user inside the container
chmod 644 ./*.key ./*.crt ./*.pem
echo "certificates in $(pwd)"
