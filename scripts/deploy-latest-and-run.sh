#!/bin/sh
image_name="untagged6785/ha-doorbell"
container_name="ha-doorbell"

docker pull $image_name:latest

echo "Stopping container $container_name"
docker stop $container_name || true

echo "Removing container $container_name"
docker rm $container_name || true

echo "Starting new container"
docker run -d --name $container_name -e HASS_TOKEN -e AWS_ACCESS_KEY_ID -e AWS_SECRET_ACCESS_KEY --restart=unless-stopped $image_name:latest

# run this via /etc/config/crontab and not via crontab -e, because QNAP...
# crontab /etc/config/crontab && /etc/init.d/crond.sh restart