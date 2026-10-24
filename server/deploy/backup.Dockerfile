# The image of the backup service of docker-compose.yml: Alpine with the
# PostgreSQL 17 client and age, installed once when the image is built
# (`docker compose build backup`, also by `docker compose up` the first
# time), so a nightly backup needs no network and no package mirror.
FROM alpine:3.22
RUN apk add --no-cache postgresql17-client age
COPY backup.sh /usr/local/bin/backup.sh
ENTRYPOINT ["/bin/sh", "/usr/local/bin/backup.sh"]
