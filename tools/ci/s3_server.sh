#!/usr/bin/env bash
# An S3 server on this machine for the shared worlds tests, with the bucket made. Tried in turn,
# since none has a home that is sure to stay: MinIO's last open-source release built from source,
# SeaweedFS, LocalStack. The tests themselves check that the one that started enforces
# conditional writes. Writes the SHARED_WORLDS_S3_* variables to $GITHUB_ENV.
set -uo pipefail
user=sharedworlds
secret=sharedworlds-secret
bucket=shared-worlds-test
export AWS_ACCESS_KEY_ID=$user AWS_SECRET_ACCESS_KEY=$secret AWS_DEFAULT_REGION=us-east-1
work=${RUNNER_TEMP:-/tmp}

# Whether the server at $1 takes a bucket: it is up, and it accepts this key.
usable() {
  for _ in $(seq 45); do
    if aws --endpoint-url "$1" s3 mb "s3://$bucket" > "$work/mb.log" 2>&1 ||
        grep -q "BucketAlreadyOwnedByYou" "$work/mb.log"; then
      return 0
    fi
    sleep 1
  done
  tail -n 3 "$work/mb.log"
  return 1
}

minio_from_source() {
  GOBIN="$work/bin" go install github.com/minio/minio@RELEASE.2025-04-22T22-12-26Z > "$work/minio-build.log" 2>&1 ||
    { tail -n 5 "$work/minio-build.log"; return 1; }
  mkdir -p "$work/minio-data"
  MINIO_ROOT_USER=$user MINIO_ROOT_PASSWORD=$secret nohup "$work/bin/minio" server \
    --address 127.0.0.1:9000 "$work/minio-data" > "$work/minio.log" 2>&1 &
  usable http://127.0.0.1:9000 && endpoint=http://127.0.0.1:9000
}

seaweedfs() {
  docker run -d --name seaweedfs -p 8333:8333 -e AWS_ACCESS_KEY_ID=$user -e AWS_SECRET_ACCESS_KEY=$secret \
    chrislusf/seaweedfs server -s3 > /dev/null 2>&1 || return 1
  usable http://127.0.0.1:8333 && endpoint=http://127.0.0.1:8333 && return 0
  docker rm -f seaweedfs > /dev/null 2>&1
  return 1
}

localstack() {
  docker run -d --name localstack -p 4566:4566 -e SERVICES=s3 localstack/localstack > /dev/null 2>&1 || return 1
  usable http://127.0.0.1:4566 && endpoint=http://127.0.0.1:4566 && return 0
  docker rm -f localstack > /dev/null 2>&1
  return 1
}

endpoint=""
for server in minio_from_source seaweedfs localstack; do
  echo "Trying $server"
  if $server; then
    echo "S3 server: $server at $endpoint"
    break
  fi
done
if [ -z "$endpoint" ]; then
  echo "No S3 server could be started here" >&2
  exit 1
fi
{
  echo "SHARED_WORLDS_S3_ENDPOINT=$endpoint"
  echo "SHARED_WORLDS_S3_BUCKET=$bucket"
  echo "SHARED_WORLDS_S3_KEY=$user"
  echo "SHARED_WORLDS_S3_SECRET=$secret"
} >> "$GITHUB_ENV"
