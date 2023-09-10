# MarcDigital
This project aims to create a digital frame for old people where the rest of the family can easily push photos.

## Functional Requirements
- Synch with a Google Fotos album. One for now, maybe more in the future.
- Have two buttons, one to move to the previous image, one to move to the following image. 
- A future button might be added to change the album
- Make the images rotate automatically

## Software requirements
- This project was built and tested in Python 3.9.2, but at least the packaging and install works in 3.10-3.11
- You will have to install GTK. For debian, you can do:
```
sudo apt install libgirepository1.0-dev gcc libcairo2-dev pkg-config python3-dev gir1.2-gtk-4.0
```
But the [official GI site](https://pygobject.readthedocs.io/en/latest/getting_started.html) has information in other OS.

## Getting started
Because this is aimed to old people, the app needs to auto-star with the raspberry, and give no options to access any file. For this 
```
mkdir -p /home/pi/.config/autostart
cp marcdigital.desktop /home/pi/.config/autostart/marcdigital.desktop
```

This will start the app. In this first iteration I am doing the synching of the pictures separately.