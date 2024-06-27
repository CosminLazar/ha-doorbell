# HA-Doorbell

Connects to HomeAssistant via websocket and monitors a sensor (doorbell) for state change. 
When someones rings the doorbell (aka presses the button):
 * a sound is played via the mediaplayer service 
 * a picture is taken from one of the video surveilance cameras and uploaded to s3
 * a push notification gets sent that contains the image (the image url is pre-signed and it will expire after a timeout)

Needless to say the whole thing is coded in "ghetto-code" style in an evening.

## Remember to
 * [import the IoS sounds into the HomeAssistant companion app](https://companion.home-assistant.io/docs/notifications/notification-sounds#importing-sounds-from-ios)
 * [create a notification group called `all_mobile_phones` for all the phones that should be notified](https://companion.home-assistant.io/docs/notifications/notifications-basic/#sending-notifications-to-multiple-devices)

## Environment variables
 * `HASS_TOKEN`
 * `AWS_ACCESS_KEY_ID`
 * `AWS_SECRET_ACCESS_KEY`
