FROM sigp/lighthouse:v5.2.1

## Add the wait script to the image
COPY --from=ghcr.io/ufoscout/docker-compose-wait:latest /wait /wait

COPY l1-lighthouse-bn-entrypoint.sh /entrypoint-bn.sh
COPY l1-lighthouse-vc-entrypoint.sh /entrypoint-vc.sh

VOLUME ["/db"]
