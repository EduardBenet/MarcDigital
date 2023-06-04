import os
import pickle
import requests
import json
from google_auth_oauthlib.flow import InstalledAppFlow
from googleapiclient.discovery import build
import google.auth

CLIENT_SECRET_FILE = '/home/pi/Documents/MarcDigital/gphotos_credentials.json'

# Google Photos API scope
SCOPES = ['https://www.googleapis.com/auth/photoslibrary.readonly']

def get_tokens():
    flow = InstalledAppFlow.from_client_secrets_file(CLIENT_SECRET_FILE, SCOPES)
    creds = flow.run_local_server(port=0)
    with open('token.pickle', 'wb') as token:
        pickle.dump(creds, token)
    print(f"Access Token: {creds.token}")
    print(f"Refresh Token: {creds.token}")

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
        'refresh_token': refresh_token.token,
        'grant_type': 'refresh_token'
    }
    response = requests.post('https://oauth2.googleapis.com/token', data = data)
    if response.status_code == 200:
        return response.json()['access_token']
    else:
        print(f"Failed to get access token: {response.token}")
        return None

def get_google_photos_service():
    
    access_token = get_access_token()

    credentials = google.oauth2.credentials.Credentials(access_token)
            
    return build('photoslibrary', 'v1', credentials=credentials, static_discovery=False)

def sync_google_photos_album(album_id, download_directory):
    service = get_google_photos_service()
    
    # Loop over pages until empty
    nextpagetoken = None
    while True:        
        # Search the album based on ID (TODO: Find out how to swap id by title)
        results = service.mediaItems().search(
            body={'albumId': album_id, "pageSize": 100, "pageToken": nextpagetoken}
        ).execute()
        items = results.get('mediaItems', [])

        # Download the images 
        if not items:
            print('No images found.')
            break
        for item in items:
            # Get new image name and url
            file_name = item['filename']
            file_url = item['baseUrl']

            # TODO: remove old images from folder

            # Download the image if does not exist yet
            newImage = os.path.join(download_directory, file_name)
            if not os.path.exists(newImage):
                download_image(file_url, newImage)

        nextpagetoken = results.get('nextPageToken', None)
        if not nextpagetoken:
            break

def download_image(url, file_path):
    response = requests.get(url)
    with open(file_path, 'wb') as file:
        file.write(response.content)

if __name__ == '__main__':
    download_directory = '/home/pi/Documents/MarcDigital/images'
    os.makedirs(download_directory, exist_ok=True)
    sync_google_photos_album('', download_directory)