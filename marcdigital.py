import os
import gi

import RPi.GPIO as GPIO

# Set up the GPIO mode to use the BCM numbering
GPIO.setmode(GPIO.BCM)

gi.require_version("Gtk", "3.0")
from gi.repository import Gtk, Gdk, GdkPixbuf

class ImageGallery(Gtk.Window):

    def __init__(self):
        super().__init__()
        # Initialize screen info for reuse
        self._get_screen_info()
        #self.fullscreen()
        
        # Add destroy callback when shutdown
        self.connect("destroy", Gtk.main_quit)
        self.connect("destroy", lambda w: GPIO.cleanup())
        self.connect("key-press-event", self.on_key_press_event)              
        
        # Images will be synched in folder images by a separate process, grab and sort them
        self.image_folder = "images"
        self.image_files = sorted([f for f in os.listdir(self.image_folder)])
        self.current_image = 0

        # Create an overlay to statck the image and the spinner widgets
        self.overlay = Gtk.Overlay()
        self.image = Gtk.Image()
        self.overlay.add(self.image)

        # Create a  spinner and initially hide it
        self.spinner = Gtk.Spinner()
        self.spinner.hide()
        self.overlay.add_overlay(self.spinner) # Add the spinner widget to the overlay

        # Initialize an image object and load the first image    
        self.load_image()

        # Initialize the buttons left, right and home (home is un-used at the moment)
        self.prev_button = Gtk.Button.new_from_icon_name("go-previous", Gtk.IconSize.BUTTON)
        self.prev_button.connect("clicked", self.on_prev_button_clicked)

        self.next_button = Gtk.Button.new_from_icon_name("go-next", Gtk.IconSize.BUTTON)
        self.next_button.connect("clicked", self.on_next_button_clicked)

        self.home_button = Gtk.Button.new_from_icon_name("go-home", Gtk.IconSize.BUTTON)
        self.home_button.connect("clicked", self.on_home_button_clicked)

        self.button_box = Gtk.Box(orientation = Gtk.Orientation.HORIZONTAL)
        self.button_box.pack_start(self.prev_button, True, True, 0) # adds button to button box container with fill extra space, make button fill all space, pixles between button and box      
        self.button_box.pack_start(self.home_button, True, True, 0)
        self.button_box.pack_start(self.next_button, True, True, 0)

        # Connect the Raspberry pi buttons to the gallery
        self.connectRPIbuttons(17,27)  

        # Create the main Gtk Box and add both image and buttons
        self.main_box = Gtk.Box(orientation=Gtk.Orientation.VERTICAL)
        self.main_box.pack_start(self.overlay, True, True, 0)
        self.main_box.pack_start(self.button_box, False, False, 0)

        self.add(self.main_box)
        self.show_all()

    def _get_screen_info(self):
        monitor = Gdk.Display.get_default().get_primary_monitor()
        geometry = monitor.get_geometry()
        scale_factor = monitor.get_scale_factor()
        self._width = scale_factor*geometry.width
        self._height = scale_factor*geometry.height- 50 # to comodate space for buttons

    def connectRPIbuttons(self, leftPin, rightPin):
        # Set up the GPIO mode to use the BCM numbering
        GPIO.setup(leftPin, GPIO.IN, pull_up_down=GPIO.PUD_UP)
        GPIO.setup(rightPin, GPIO.IN, pull_up_down=GPIO.PUD_UP)

        # add an event, on the desired pin while rising
        # To avoid switch bouncing, which can cause edge RISING to be detected more than once, we add bouncetime
        GPIO.add_event_detect(leftPin, GPIO.RISING, callback = lambda channel: self.on_prev_button_clicked(None), bouncetime = 1000)
        GPIO.add_event_detect(rightPin, GPIO.RISING, callback = lambda channel: self.on_next_button_clicked(None), bouncetime = 1000)m

    def on_key_press_event(self, widget, event):
        if event.keyval == Gdk.KEY_Escape: # Check if key presed is ESC
            self.destroy() #Close the application
        elif event.keyval == Gdk.KEY_Left:
            self.on_prev_button_clicked(None)
        elif event.keyval == Gdk.KEY_Right:
            self.on_next_button_clicked(None)

    def load_image(self):   

        self.spinner.show()     
        self.spinner.start()
        self.image.hide()

        pixbuf = GdkPixbuf.Pixbuf.new_from_file_at_scale(
            os.path.join(self.image_folder, self.image_files[self.current_image]), 
            self._width, 
            self._height, 
            preserve_aspect_ratio=True,
        )

        self.image.set_from_pixbuf(pixbuf)
        self.image.show()
        self.spinner.stop()
        self.spinner.hide()

        
    def on_prev_button_clicked(self, button):
        self.current_image -= 1
        if self.current_image < 0:
            self.current_image = len(self.image_files) - 1
        self.load_image()

    def on_next_button_clicked(self, button):
        self.current_image += 1
        if self.current_image >= len(self.image_files):
            self.current_image = 0
        self.load_image()

    def on_home_button_clicked(self, button):
        self.current_image = 0
        self.load_image()
        
def main():
    app = ImageGallery()
    Gtk.main()

if __name__=="__main__":
    main()