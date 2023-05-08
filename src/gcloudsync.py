import os
import pickle
import requests
from google_auth_oauthlib.flow import InstalledAppFlow
from google.auth.transport.requests import Request
from googleapiclient.discovery import build

CLIENT_SECRET_FILE = '/home/pi/Documents/MarcDigital/gphotos_credentials.json'

# Google Photos API scope
SCOPES = ['https://www.googleapis.com/auth/photoslibrary.readonly']

def get_google_photos_service():
    creds = None

    if os.path.exists('token.pickle'):
        with open('token.pickle', 'rb') as token:
            creds = pickle.load(token)

    if not creds or not creds.valid:
        if creds and creds.expired and creds.refresh_token:
            creds.refresh(Request())
        else:
            flow = InstalledAppFlow.from_client_secrets_file(CLIENT_SECRET_FILE, SCOPES)
            creds = flow.run_local_server(port=0)
        with open('token.pickle', 'wb') as token:
            pickle.dump(creds, token)
            
    return build('photoslibrary', 'v1', credentials=creds, static_discovery=False)

def sync_google_photos_album(album_id, download_directory):
    service = get_google_photos_service()
    nextpagetoken = None
    while True:
        """ results = service.albums().list(
            pageSize = 50, pageToken = nextpagetoken, fields="nextPageToken,albums(id)"
        ).execute()
        albums = results.get('albums', [])
        for album in albums:
            print(f'Album ID: {album["id"]}')
        nextpagetoken = results.get('nextPageToken', None)
        if not nextpagetoken:
            break """
        
        results = service.mediaItems().search(
            body={'albumId': album_id, "pageSize": 100, "pageToken": nextpagetoken}
        ).execute()
        items = results.get('mediaItems', [])
        if not items:
            print('No images found.')
            break
        for item in items:
            file_name = item['filename']
            file_url = item['baseUrl']
            download_image(file_url, os.path.join(download_directory, file_name))
            print(file_name)

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