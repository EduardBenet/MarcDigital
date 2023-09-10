#!/bin/python

import os, json, pickle, requests, shutil

from datetime import timezone

from typing import List, Optional

from tempfile import NamedTemporaryFile
# Google Auth
from google_auth_oauthlib.flow import InstalledAppFlow
from googleapiclient.discovery import build
import google.auth

from requests.adapters import HTTPAdapter
from requests_oauthlib import OAuth2Session
from urllib3.util.retry import Retry

from json import dump, load, JSONDecodeError

from pathlib import Path

# OAuth endpoints given in the Google API documentation
authorization_base_url = "https://accounts.google.com/o/oauth2/v2/auth"
token_uri = "https://www.googleapis.com/oauth2/v4/token"

CLIENT_SECRET_FILE = '/home/pi/Documents/MarcDigital/gphotos_credentials.json'

# Google Photos API scope
SCOPES = ['https://www.googleapis.com/auth/photoslibrary.readonly']

class gcloud_auth:
    def __init__(
        self,
        secrets_file: str,
        token_file: str,
        port: int = 8080,
    ):
        self.token_file = Path(token_file)
        self.session = None
        self.token = None
        self.secrets_file = Path(secrets_file)
        self.port = port

        try:
            with self.secrets_file.open('r') as stream:
                g_creds = load(stream)

            secrets = g_creds["installed"]
            self.client_id = secrets["client_id"]
            self.client_secret = secrets["client_secret"]
            self.redirect_uri = secrets["redirect_uris"][0]
            self.token_uri = secrets["token_uri"]
            self.extra = {
                "client_id": self.client_id,
                "client_secret": self.client_secret,
            }

        except (JSONDecodeError, IOError):
            print("missing or bad credentials file {}".format(secrets_file))
            exit(1)
            
    def load_token(self) -> Optional[str]:
        try:
            with self.token_file.open("r") as stream:
                token = load(stream)
        except (JSONDecodeError, IOError):
            return None
        return token

    def save_token(self, token: str):
        with self.token_file.open("w") as stream:
            dump(token, stream)
        self.token_file.chmod(0o600)
    
    def authorize(self):
        token = self.load_token()

        if token:
            self.session = OAuth2Session(
                self.client_id,
                token = token,
                auto_refresh_url = self.token_uri,
                auto_refresh_kwargs = self.extra,
                token_updater = self.save_token                
            )
        else:
            flow = InstalledAppFlow.from_client_secrets_file(
                CLIENT_SECRET_FILE, 
                SCOPES
                )            

            # localhost and bind to 0.0.0.0 always works even in a container.
            flow.run_local_server(
                open_browser=True, bind_addr="0.0.0.0", port=self.port
            )

            self.session = flow.authorized_session()

            # Mapping for backward compatibility
            oauth2_token = {
                "access_token": flow.credentials.token,
                "refresh_token": flow.credentials.refresh_token,
                "token_type": "Bearer",
                "scope": flow.credentials.scopes,
                "expires_at": flow.credentials.expiry.replace(
                    tzinfo=timezone.utc
                ).timestamp(),
            }

            self.save_token(oauth2_token)

class PhotosManager():

    def __init__(self, auth):
        auth.authorize()
        self.photos_service = self.get_google_photos_service(auth.session.access_token)

    def get_google_photos_service(self, access_token):

        credentials = google.oauth2.credentials.Credentials(access_token)

        return build('photoslibrary', 'v1', credentials=credentials, static_discovery=False)

    def sync_google_photos_album(self, album_id, download_directory, fullImage = True):

        service = self.photos_service

        # Loop over pages until empty
        nextpagetoken = None

        while True:        
            # Search the album based on ID (TODO: Find out how to swap id by title)
            results = service.mediaItems().search(
                body={'albumId': album_id, "pageSize": 100, "pageToken": nextpagetoken}
            ).execute()
            items = results.get('mediaItems', [])

            currentFiles = set(os.listdir(download_directory))
            cloudImages = set()

            # Stop if no images are found
            if not items:
                print('No images found.')
                break
            
            # Download the images 
            for item in items:
                # Get new image name and url
                file_name = item['filename']

                # For testing, it is useful to allow downloading just the image snapshot, which are much smaller
                file_url = item['baseUrl']
                if fullImage:
                    file_url = file_url + "=d"          

                cloudImages.add(file_name)

                # Download the image if does not exist yet
                newImage = os.path.join(download_directory, file_name)
                if not os.path.exists(newImage):
                    download_image(file_url, newImage)
                    # I don't want more than one download per sync. 
                    # This could potentially take a very long time and it does not really matter if we sync along an entire day.
                    break

            # Remove the files in disk no longer in the cloud
            toRemove = currentFiles - cloudImages
            for img in toRemove:
                location = os.path.join(download_directory, img)
                os.remove(location)

            nextpagetoken = results.get('nextPageToken', None)
            if not nextpagetoken:
                break

def download_image(url, file_path):
    response = requests.get(url)

    expected_size = int(response.headers.get('Content-Length', 0))
    actual_size = 0
    with NamedTemporaryFile(delete=False) as temp_file:
        for chunk in response.iter_content(1024):
            temp_file.write(chunk)
            actual_size += len(chunk)

    # Maybe check if the download was ok
    if actual_size == expected_size:
        shutil.move(temp_file.name, file_path)
      
'''
def get_tokens():
    # Get credentials interactively from OAUTH.
    flow = InstalledAppFlow.from_client_secrets_file(CLIENT_SECRET_FILE, SCOPES)
    creds = flow.run_local_server(port=0) # open_browser=False, bind_addr="0.0.0.0", port=port.

    #flow.run_local_server()
    # self.session = flow.authorized_session
    # oauth2_token = {
    #    "access_token": flow.credentials.token
    #    "refresh_token": flow.credentials.refresh_token,
    #    "scope": flow.credentials.scopes
    #    "expires_at": flow.credentials.epiry.replace(
    #       tzinfo = timezone.utc    
    # ).timestamp(),
    # }
    # self.save_token(oauth2_token)
    """ 
    # set upo the retry behaviour 
    retries = Retry(
        total = self.max_retries,
        backoff_factor = 5,
        status_forcelist = [500, 502, 503, 504, 429],
        allowed_methods = frozenet(["GET", "POST"]),
        raise_on_status = False,
        respect_retry_after_header = True,
    )

    # applyt the retry behaviour to our session by replacing the default HTTPAdapter
    self.session.mount("https://", HTTPAdapter(max_retries = retires))
    """

    with open('token.pickle', 'wb') as token:
        pickle.dump(creds, token)

def get_access_token():

    if not os.path.exists('token.pickle'):
        get_tokens()
    
    with open('token.pickle', 'rb') as token:
        refresh_token = pickle.load(token)
        
    f = open(CLIENT_SECRET_FILE)
    g_creds = json.load(f)
    data = {
        'client_id': g_creds['installed']['client_id'],
        'client_secret':  g_creds['installed']['client_secret'],
        'refresh_token': refresh_token.refresh_token,
        'grant_type': 'refresh_token'
    }
    response = requests.post('https://oauth2.googleapis.com/token', data = data)
    if response.status_code == 200:
        return response.json()['access_token']
    elif response.status_code == 400:
        get_tokens()
        return get_access_token()
    else:
        print(f"Failed to get access token: {response.token}")
        return None

def get_google_photos_service():
    
    access_token = get_access_token()

    credentials = google.oauth2.credentials.Credentials(access_token)
            
    return build('photoslibrary', 'v1', credentials=credentials, static_discovery=False)

def sync_google_photos_album(album_id, download_directory, fullImage = True):
    service = get_google_photos_service()
    
    # Loop over pages until empty
    nextpagetoken = None
    while True:        
        # Search the album based on ID (TODO: Find out how to swap id by title)
        results = service.mediaItems().search(
            body={'albumId': album_id, "pageSize": 100, "pageToken": nextpagetoken}
        ).execute()
        items = results.get('mediaItems', [])

        currentFiles = set(os.listdir(download_directory))
        cloudImages = set()

        # Download the images 
        if not items:
            print('No images found.')
            break
        for item in items:
            # Get new image name and url
            file_name = item['filename']

            # For testing, it is useful to allow downloading just the image snapshot, which are much smaller
            file_url = item['baseUrl']
            if fullImage:
                file_url = file_url + "=d"          

            cloudImages.add(file_name)

            # Download the image if does not exist yet
            newImage = os.path.join(download_directory, file_name)
            if not os.path.exists(newImage):
                download_image(file_url, newImage)
                # I don't want more than one download per sync. 
                # This could potentially take a very long time and it does not really matter if we sync along an entire day.
                break

        # Remove the files in disk no longer in the cloud
        toRemove = currentFiles - cloudImages
        for img in toRemove:
            location = os.path.join(download_directory, img)
            os.remove(location)

        nextpagetoken = results.get('nextPageToken', None)
        if not nextpagetoken:
            break
'''

if __name__ == '__main__':
    download_directory = '/home/pi/Documents/MarcDigital/images'
    os.makedirs(download_directory, exist_ok=True)

    auth_client = gcloud_auth('gphotos_credentials.json', 'token.json')

    pm = PhotosManager(auth_client)
    pm.sync_google_photos_album('AF9Qav513ch3z47nnhS2d-REj_nXfAS7f3gErmU_62VUsZPgcHYe_x56yWE0AvNxO9kG_M7BmM8D', download_directory, fullImage = True)

    # os.makedirs(download_directory, exist_ok=True)
    # sync_google_photos_album('AF9Qav513ch3z47nnhS2d-REj_nXfAS7f3gErmU_62VUsZPgcHYe_x56yWE0AvNxO9kG_M7BmM8D', download_directory, fullImage = True)