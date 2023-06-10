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
    # Get credentials interactively from OAUTH.
    flow = InstalledAppFlow.from_client_secrets_file(CLIENT_SECRET_FILE, SCOPES)
    creds = flow.run_local_server(port=0)
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

def download_image(url, file_path):
    response = requests.get(url)
    with open(file_path, 'wb') as file:
        file.write(response.content)

if __name__ == '__main__':
    download_directory = '/home/pi/Documents/MarcDigital/images'
    os.makedirs(download_directory, exist_ok=True)
    sync_google_photos_album('AF9Qav513ch3z47nnhS2d-REj_nXfAS7f3gErmU_62VUsZPgcHYe_x56yWE0AvNxO9kG_M7BmM8D', download_directory, fullImage = False)