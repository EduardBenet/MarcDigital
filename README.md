# MarcDigital
This project aims to create a digital frame for old people where the rest of the family can easily push photos.

## Functional Requirements
- Synch with a Google Fotos album. One for now, maybe more in the future.
- Have two buttons, one to move to the previous image, one to move to the following image. 
- A future button might be added to change the album
- Make the images rotate automatically

## Software requirements
- This project was built and tested in Python 3.9.2, but at least the packaging and install works in 3.10
- You will have to install GTK. For debian, you can do:
```
sudo apt install libgirepository1.0-dev gcc libcairo2-dev pkg-config python3-dev gir1.2-gtk-4.0
```
But the [official GI site](https://pygobject.readthedocs.io/en/latest/getting_started.html) has information about other OS.

Install the package from the releases:
```
pip install ./dist/MarcDigital-0.1-py3-none-any.whl
```

## Getting started
To simply start the app you can do:
```
from MarcDigital import ImageGallery
app = ImageGallery(image_rot_freq = 3)
Gtk.main()
```
Because this is aimed to be completely automated, the app needs to auto-start with the raspberry, and give no options to access any file. 

For this 
```
mkdir -p /home/pi/.config/autostart
cp marcdigital.desktop /home/pi/.config/autostart/marcdigital.desktop
```

This will start the app and start synching your images