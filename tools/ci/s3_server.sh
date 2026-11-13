#!/usr/bin/env bash
# An S3 server on this machine for the shared worlds tests, with the bucket made: MinIO from
# whichever of its homes still serves it. Writes the SHARED_WORLDS_S3_* variables to $GITHUB_ENV.
set -uo pipefail
user=sharedworlds
secret=sharedworlds-secret
bucket=shared-worlds-test
port=9000

ready() {
  for _ in $(seq 60); do
    curl -fsS "http://127.0.0.1:$port/minio/health/ready" > /dev/null 2>&1 && return 0
    sleep 1
  done
  return 1
}

started=""
for image in quay.io/minio/minio:latest minio/minio:latest; do
  if docker run -d --name minio -p "$port:9000" -e MINIO_ROOT_USER=$user -e MINIO_ROOT_PASSWORD=$secret \
      "$image" server /data > /dev/null 2>&1 && ready; then
    started="$image"
    break
  fi
  docker rm -f minio > /dev/null 2>&1
done
if [ -z "$started" ]; then
  for url in https://dl.min.io/server/minio/release/linux-amd64/minio \
             https://dl.min.io/aistor/minio/release/linux-amd64/minio; do
    if curl -fsSL --retry 2 -o "$RUNNER_TEMP/minio" "$url" && chmod +x "$RUNNER_TEMP/minio"; then
      mkdir -p "$RUNNER_TEMP/minio-data"
      MINIO_ROOT_USER=$user MINIO_ROOT_PASSWORD=$secret nohup "$RUNNER_TEMP/minio" server \
        --address "127.0.0.1:$port" "$RUNNER_TEMP/minio-data" > "$RUNNER_TEMP/minio.log" 2>&1 &
      if ready; then
        started="$url"
        break
      fi
      tail -n 20 "$RUNNER_TEMP/minio.log" || true
    fi
  done
fi
if [ -z "$started" ]; then
  echo "No S3 server could be started here" >&2
  exit 1
fi
echo "S3 server: $started"
set -e
AWS_ACCESS_KEY_ID=$user AWS_SECRET_ACCESS_KEY=$secret AWS_DEFAULT_REGION=us-east-1 \
  aws --endpoint-url "http://127.0.0.1:$port" s3 mb "s3://$bucket"
{
  echo "SHARED_WORLDS_S3_ENDPOINT=http://127.0.0.1:$port"
  echo "SHARED_WORLDS_S3_BUCKET=$bucket"
  echo "SHARED_WORLDS_S3_KEY=$user"
  echo "SHARED_WORLDS_S3_SECRET=$secret"
} >> "$GITHUB_ENV"
