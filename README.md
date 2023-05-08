# MarcDigital
This project aims to create a digital frame for old people where the rest of the family can easily push photos.

## Requirements
- Synch with a Google Fotos album. One for now, maybe more in the future.
- Have two buttons, one to move to the previous image, one to move to the following image. 
- A future button might be added to change the album
- Optionally, make the images change automatically.
- No touch screen, buttons only

## Getting started
Because this is aimed to old people, the app needs to auto-star with the raspberry, and give no options to access any file. For this 
```
mkdir -p /home/pi/.config/autostart
cp marcdigital.desktop /home/pi/.config/autostart/marcdigital.desktop
```